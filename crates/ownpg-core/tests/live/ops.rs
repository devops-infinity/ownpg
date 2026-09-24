use crate::support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ownpg_core::audit::{Sink, Transport};
use ownpg_core::config::{FlagLayer, Mode, ToolGroup};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, stdio};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientConfig, ProgressNotificationParam, ProgressToken,
    RequestMetaObject,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
struct ProgressCounter {
    progress: Arc<AtomicUsize>,
}

impl ClientHandler for ProgressCounter {
    async fn on_progress(
        &self,
        notification: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        assert!(notification.progress >= 0.0);
        self.progress.fetch_add(1, Ordering::SeqCst);
    }

    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

struct Rig {
    client: RunningService<RoleClient, ProgressCounter>,
    server_task: tokio::task::JoinHandle<ownpg_core::Result<ownpg_core::ExitClass>>,
    progress: Arc<AtomicUsize>,
}

async fn rig(scratch: &support::Scratch, mode: Mode, groups: Vec<ToolGroup>) -> Rig {
    let client = scratch.client().await;
    client
        .batch_execute(
            "CREATE SCHEMA app; \
             CREATE TABLE app.events (id bigint generated always as identity primary key, payload text, at timestamptz default now()); \
             INSERT INTO app.events (payload) SELECT repeat('x', 200) FROM generate_series(1, 20000); \
             DELETE FROM app.events WHERE id % 2 = 0; \
             CREATE INDEX events_at_idx ON app.events (at); \
             CREATE INDEX events_at_dup_idx ON app.events (at); \
             CREATE UNIQUE INDEX events_id_unique ON app.events (id); \
             CREATE INDEX events_id_idx ON app.events (id); \
             CREATE MATERIALIZED VIEW app.event_counts AS SELECT count(*) AS n FROM app.events;",
        )
        .await
        .unwrap();
    let settings = Arc::new(scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        mode: Some(mode),
        tools: Some(groups),
        ..FlagLayer::default()
    }));
    let engine = Engine::start(Arc::clone(&settings), Hints::default())
        .await
        .expect("the engine starts");
    let server = Arc::new(
        Server::new(
            Arc::new(engine),
            Arc::new(Sink::disabled()),
            Transport::Stdio,
            Principal::local(Some("ops")),
        )
        .await
        .expect("the server builds"),
    );
    let (client_side, server_side) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server_task = tokio::spawn(stdio::serve(server, server_read, server_write));
    let (client_read, client_write) = tokio::io::split(client_side);
    let handler = ProgressCounter::default();
    let progress = Arc::clone(&handler.progress);
    let client = handler
        .serve_with_lifecycle((client_read, client_write), ClientLifecycleMode::Initialize)
        .await
        .expect("the client handshake completes");
    Rig {
        client,
        server_task,
        progress,
    }
}

impl Rig {
    async fn call(&self, tool: &str, arguments: Value) -> CallToolResult {
        let object = arguments.as_object().cloned().unwrap_or_default();
        self.client
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(object))
            .await
            .expect("the call returns a result")
    }

    async fn ok(&self, tool: &str, arguments: Value) -> Value {
        let result = self.call(tool, arguments).await;
        assert_ne!(result.is_error, Some(true), "{tool}: {result:?}");
        result.structured_content.unwrap_or(Value::Null)
    }

    async fn finish(self) {
        drop(self.client);
        self.server_task
            .await
            .unwrap()
            .expect("the server stops cleanly");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintenance_tools_vacuum_analyze_reindex_and_refresh_with_progress() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let rig = rig(
        &scratch,
        Mode::ReadWrite,
        vec![ToolGroup::Maintenance, ToolGroup::Monitoring],
    )
    .await;
    let listed = rig.client.list_tools(None).await.unwrap();
    let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    for expected in [
        "pg_vacuum",
        "pg_analyze",
        "pg_reindex",
        "pg_refresh",
        "pg_vacuum_needs",
        "pg_backend",
        "pg_activity",
        "pg_locks",
        "pg_replication",
        "pg_wal",
        "pg_indexes_health",
        "pg_bloat",
        "pg_settings",
        "pg_top_queries",
    ] {
        assert!(names.contains(&expected), "{expected} missing");
    }

    let needs = rig.ok("pg_vacuum_needs", json!({})).await;
    let events = needs["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["table"] == "events")
        .expect("the events table is listed");
    assert!(events["dead_rows"].as_i64().unwrap() >= 0);
    assert!(events["transaction_id_age"].as_i64().unwrap() >= 0);
    assert!(events["inserted_since_vacuum"].as_i64().unwrap() >= 0);
    assert!(
        events["insert_threshold"].as_i64().unwrap() >= 1000,
        "{events}"
    );

    let dry = rig
        .ok(
            "pg_vacuum",
            json!({"tables": ["events"], "analyze": true, "skip_locked": true, "dry_run": true}),
        )
        .await;
    assert_eq!(
        dry["sql"],
        "VACUUM (ANALYZE, SKIP_LOCKED) \"app\".\"events\""
    );

    let mut meta = RequestMetaObject::default();
    meta.set_progress_token(ProgressToken(rmcp::model::NumberOrString::Number(7)));
    let mut params = CallToolRequestParams::new("pg_vacuum").with_arguments(
        json!({"tables": ["events"], "analyze": true})
            .as_object()
            .cloned()
            .unwrap(),
    );
    params.meta = Some(meta);
    let vacuumed = rig.client.call_tool(params).await.unwrap();
    assert_ne!(vacuumed.is_error, Some(true), "{vacuumed:?}");

    let full_needs_confirm = rig
        .call("pg_vacuum", json!({"tables": ["events"], "full": true}))
        .await;
    assert_eq!(
        full_needs_confirm.structured_content.unwrap()["code"],
        "confirmation.required"
    );
    rig.ok(
        "pg_vacuum",
        json!({"tables": ["events"], "full": true, "confirm": true}),
    )
    .await;

    rig.ok(
        "pg_analyze",
        json!({"tables": ["events"], "columns": ["payload"]}),
    )
    .await;
    rig.ok("pg_reindex", json!({"target": "table", "name": "events"}))
        .await;
    rig.ok(
        "pg_reindex",
        json!({"target": "schema", "concurrently": true}),
    )
    .await;
    rig.ok("pg_refresh", json!({"view": "event_counts"})).await;
    let counted = rig
        .ok("pg_run_query", json!({"sql": "SELECT n FROM event_counts"}))
        .await;
    assert_eq!(counted["rows"][0][0], 10000);

    let health = rig.ok("pg_indexes_health", json!({})).await;
    let problems: Vec<String> = health["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row[0].as_str().unwrap().to_owned())
        .collect();
    assert!(problems.contains(&"duplicate".to_owned()), "{problems:?}");
    let duplicates: Vec<(String, String)> = health["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row[0] == "duplicate")
        .map(|row| {
            (
                row[2].as_str().unwrap().to_owned(),
                row[4].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let advice_for = |index: &str| {
        duplicates
            .iter()
            .find(|(name, _)| name == index)
            .map(|(_, advice)| advice.clone())
    };
    assert!(advice_for("events_pkey").is_none(), "{duplicates:?}");
    assert_eq!(
        advice_for("events_id_unique").as_deref(),
        Some("same columns as events_pkey"),
        "{duplicates:?}"
    );
    assert!(
        advice_for("events_id_idx").is_some_and(|advice| advice.contains("events_pkey")),
        "{duplicates:?}"
    );

    let bloat = rig.ok("pg_bloat", json!({})).await;
    assert!(bloat["row_count"].as_u64().unwrap() >= 1);
    let exact = rig.call("pg_bloat", json!({"exact_table": "events"})).await;
    let body = exact.structured_content.unwrap();
    assert!(
        body["code"] == "extension.missing" || body["row_count"].as_u64().unwrap() >= 1,
        "{body}"
    );

    let activity = rig.ok("pg_activity", json!({"include_idle": true})).await;
    assert!(activity["columns"].as_array().unwrap().len() >= 10);
    rig.ok("pg_locks", json!({})).await;
    let replication = rig.ok("pg_replication", json!({})).await;
    assert_eq!(replication["rows"][0][1], "in_recovery");
    let wal = rig.ok("pg_wal", json!({})).await;
    assert!(wal["row_count"].as_u64().unwrap() >= 5);
    let settings = rig.ok("pg_settings", json!({"pattern": "work_mem"})).await;
    assert_eq!(settings["rows"][0][0], "work_mem");
    let top = rig.call("pg_top_queries", json!({})).await;
    let body = top.structured_content.unwrap();
    assert!(
        body["code"] == "extension.missing" || body["row_count"].is_number(),
        "{body}"
    );

    let cancel_dry_run = rig
        .ok(
            "pg_backend",
            json!({"operation": "cancel", "pid": 1, "dry_run": true}),
        )
        .await;
    assert_eq!(
        cancel_dry_run["sql"],
        "SELECT pg_catalog.pg_cancel_backend(1) AS signalled"
    );
    let terminate = rig
        .call("pg_backend", json!({"operation": "terminate", "pid": 1}))
        .await;
    assert_eq!(
        terminate.structured_content.unwrap()["code"],
        "confirmation.required"
    );
    assert!(rig.progress.load(Ordering::SeqCst) >= 1);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitoring_loads_in_read_only_mode_and_maintenance_does_not() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let rig = rig(&scratch, Mode::ReadOnly, vec![ToolGroup::Monitoring]).await;
    let listed = rig.client.list_tools(None).await.unwrap();
    let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert!(names.contains(&"pg_activity"));
    assert!(!names.contains(&"pg_vacuum"));
    let settings = rig.ok("pg_settings", json!({"changed_only": true})).await;
    assert!(settings["row_count"].is_number());
    rig.finish().await;
}
