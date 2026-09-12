#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;

use ownpg_core::audit::{Sink, Transport};
use ownpg_core::config::{FlagLayer, Mode};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, stdio};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, ElicitRequestParams,
    ElicitResult, ElicitationAction, Implementation, ProtocolVersion,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, ErrorData, RoleClient};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct Confirming {
    proceed: bool,
}

impl ClientHandler for Confirming {
    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        let ElicitRequestParams::FormElicitationParams { message, .. } = request else {
            return Ok(ElicitResult::new(ElicitationAction::Cancel));
        };
        assert!(message.contains("destructive"), "{message}");
        Ok(ElicitResult::new(ElicitationAction::Accept)
            .with_content(json!({"proceed": self.proceed})))
    }

    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::builder().enable_elicitation().build(),
            Implementation::new("write-test", "0"),
        )
        .with_protocol_version(ProtocolVersion::V_2026_07_28)
    }
}

struct Rig<C: rmcp::service::Service<RoleClient>> {
    engine: Arc<Engine>,
    client: RunningService<RoleClient, C>,
    server_task: tokio::task::JoinHandle<ownpg_core::Result<ownpg_core::ExitClass>>,
    data_dir: std::path::PathBuf,
}

async fn prepare(scratch: &support::Scratch) {
    let client = scratch.client().await;
    client
        .batch_execute(
            "CREATE SCHEMA app; \
             CREATE TABLE app.items (id int primary key, name text not null, qty int default 0, tags text[]); \
             INSERT INTO app.items (id, name, qty) VALUES (1, 'one', 1), (2, 'two', 2), (3, 'three', 3); \
             CREATE SCHEMA other; CREATE TABLE other.secrets (id int);",
        )
        .await
        .unwrap();
}

async fn engine_for(scratch: &support::Scratch, mode: Mode) -> (Arc<Engine>, Arc<Sink>) {
    let settings = Arc::new(scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        mode: Some(mode),
        ..FlagLayer::default()
    }));
    let audit = Arc::new(
        Sink::open(&settings.audit, &settings.paths.data_dir, None).expect("the audit sink opens"),
    );
    let engine = Engine::start(Arc::clone(&settings), Hints::default())
        .await
        .expect("the engine starts");
    (Arc::new(engine), audit)
}

async fn serve<C>(
    engine: Arc<Engine>,
    audit: Arc<Sink>,
    principal: &str,
    handler: C,
    lifecycle: ClientLifecycleMode,
) -> Rig<C>
where
    C: rmcp::service::Service<RoleClient>,
{
    let data_dir = audit
        .path()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap();
    let server = Arc::new(
        Server::new(
            Arc::clone(&engine),
            audit,
            Transport::Stdio,
            Principal::local(Some(principal)),
        )
        .await
        .expect("the server builds"),
    );
    let (client_side, server_side) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server_task = tokio::spawn(stdio::serve(server, server_read, server_write));
    let (client_read, client_write) = tokio::io::split(client_side);
    let client = handler
        .serve_with_lifecycle((client_read, client_write), lifecycle)
        .await
        .expect("the client handshake completes");
    Rig {
        engine,
        client,
        server_task,
        data_dir,
    }
}

impl<C: rmcp::service::Service<RoleClient>> Rig<C> {
    async fn call(&self, tool: &str, arguments: Value) -> CallToolResult {
        let object = arguments.as_object().cloned().unwrap_or_default();
        self.client
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(object))
            .await
            .expect("the call returns a result")
    }

    async fn finish(self) {
        drop(self.client);
        self.server_task
            .await
            .unwrap()
            .expect("the server stops cleanly");
        drop(self.engine);
    }
}

fn structured(result: &CallToolResult) -> Value {
    result.structured_content.clone().unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_typed_write_tools_insert_update_delete_merge_and_copy() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let (engine, audit) = engine_for(&scratch, Mode::ReadWrite).await;
    let rig = serve(engine, audit, "writer", (), ClientLifecycleMode::Initialize).await;

    let inserted = rig
        .call(
            "pg_insert",
            json!({"table": "items", "rows": [{"id": 4, "name": "four", "tags": ["a", "b"]}, {"id": 5, "name": "five", "qty": 5}], "returning": ["id", "qty"]}),
        )
        .await;
    assert_ne!(inserted.is_error, Some(true), "{inserted:?}");
    let body = structured(&inserted);
    assert_eq!(body["rows_affected"], 2);
    assert_eq!(body["rows"][0][0], "4");
    assert_eq!(body["rows"][0][1], Value::Null);
    assert_eq!(body["rows"][1][1], "5");

    let dry = rig
        .call(
            "pg_insert",
            json!({"table": "items", "rows": [{"id": 9, "name": "nine"}], "dry_run": true}),
        )
        .await;
    let body = structured(&dry);
    assert_eq!(body["dry_run"], true);
    assert!(
        body["sql"]
            .as_str()
            .unwrap()
            .starts_with("INSERT INTO \"app\".\"items\"")
    );

    let upsert = rig
        .call(
            "pg_insert",
            json!({"table": "items", "rows": [{"id": 4, "name": "four again", "qty": 44}], "on_conflict": "update", "conflict_columns": ["id"], "returning": ["*"]}),
        )
        .await;
    assert_ne!(upsert.is_error, Some(true), "{upsert:?}");
    assert_eq!(structured(&upsert)["rows"][0][1], "four again");

    let updated = rig
        .call(
            "pg_update",
            json!({"table": "items", "set": {"qty": 10}, "filter": "id <= 2", "returning": ["id", "qty"]}),
        )
        .await;
    assert_ne!(updated.is_error, Some(true), "{updated:?}");
    assert_eq!(structured(&updated)["rows_affected"], 2);

    let deleted = rig
        .call(
            "pg_delete",
            json!({"table": "items", "filter": "id = 5", "returning": ["name"]}),
        )
        .await;
    assert_eq!(structured(&deleted)["rows"][0][0], "five");

    let merged = rig
        .call(
            "pg_merge",
            json!({"table": "items", "rows": [{"id": 3, "name": "three merged", "qty": 33}, {"id": 6, "name": "six", "qty": 6}], "match_on": ["id"], "returning": ["id"]}),
        )
        .await;
    assert_ne!(merged.is_error, Some(true), "{merged:?}");
    assert_eq!(structured(&merged)["rows_affected"], 2);

    let raw = rig
        .call(
            "pg_run_write",
            json!({"sql": "INSERT INTO items (id, name) VALUES (7, 'seven') RETURNING id"}),
        )
        .await;
    assert_ne!(raw.is_error, Some(true), "{raw:?}");
    assert_eq!(structured(&raw)["rows"][0][0], "7");

    let read_refused = rig.call("pg_run_write", json!({"sql": "SELECT 1"})).await;
    assert_eq!(read_refused.is_error, Some(true));
    assert!(
        structured(&read_refused)["rule"]
            .as_str()
            .unwrap()
            .contains("pg_run_query")
    );

    let ddl_refused = rig
        .call(
            "pg_run_write",
            json!({"sql": "ALTER TABLE items ADD COLUMN x int"}),
        )
        .await;
    assert_eq!(structured(&ddl_refused)["code"], "statement.refused");

    let outside = rig
        .call(
            "pg_insert",
            json!({"table": "other.secrets", "rows": [{"id": 1}]}),
        )
        .await;
    assert_eq!(structured(&outside)["code"], "statement.refused");

    let injected = rig
        .call(
            "pg_delete",
            json!({"table": "items", "filter": "id = 1); DROP TABLE items; --"}),
        )
        .await;
    assert_eq!(injected.is_error, Some(true), "{injected:?}");

    let loaded = rig
        .call(
            "pg_copy",
            json!({"direction": "in", "table": "items", "columns": ["id", "name", "qty"], "format": "csv", "header": true, "data": "id,name,qty\n20,twenty,20\n21,twenty one,21\n"}),
        )
        .await;
    assert_ne!(loaded.is_error, Some(true), "{loaded:?}");
    assert_eq!(structured(&loaded)["rows_loaded"], 2);

    let dumped = rig
        .call(
            "pg_copy",
            json!({"direction": "out", "query": "SELECT id, name FROM items WHERE id >= 20 ORDER BY id", "format": "csv", "header": true}),
        )
        .await;
    assert_ne!(dumped.is_error, Some(true), "{dumped:?}");
    let text = structured(&dumped)["data"].as_str().unwrap().to_owned();
    assert!(
        text.starts_with("id,name\n20,twenty\n21,twenty one\n"),
        "{text}"
    );

    let binary = rig
        .call(
            "pg_copy",
            json!({"direction": "out", "table": "items", "format": "binary"}),
        )
        .await;
    assert_eq!(structured(&binary)["code"], "argument.invalid");

    let count = rig
        .call("pg_run_query", json!({"sql": "SELECT count(*) FROM items"}))
        .await;
    assert_eq!(structured(&count)["rows"][0][0], "8");
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destructive_statements_need_confirm_without_elicitation_and_are_prompted_with_it() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let (engine, audit) = engine_for(&scratch, Mode::ReadWrite).await;
    let older = serve(
        Arc::clone(&engine),
        Arc::clone(&audit),
        "writer",
        (),
        ClientLifecycleMode::Initialize,
    )
    .await;
    let refused = older.call("pg_delete", json!({"table": "items"})).await;
    assert_eq!(refused.is_error, Some(true));
    let body = structured(&refused);
    assert_eq!(body["code"], "confirmation.required");
    assert!(body["remedy"].as_str().unwrap().contains("confirm"));
    let raw = older
        .call(
            "pg_run_write",
            json!({"sql": "UPDATE items SET qty = 0 WHERE true"}),
        )
        .await;
    assert_eq!(structured(&raw)["code"], "confirmation.required");
    let confirmed = older
        .call(
            "pg_update",
            json!({"table": "items", "set": {"qty": 0}, "confirm": true}),
        )
        .await;
    assert_ne!(confirmed.is_error, Some(true), "{confirmed:?}");
    assert_eq!(structured(&confirmed)["rows_affected"], 3);
    older.finish().await;

    let declining = serve(
        Arc::clone(&engine),
        Arc::clone(&audit),
        "writer",
        Confirming { proceed: false },
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await;
    let declined = declining.call("pg_delete", json!({"table": "items"})).await;
    assert_eq!(declined.is_error, Some(true), "{declined:?}");
    assert!(
        structured(&declined)["rule"]
            .as_str()
            .unwrap()
            .contains("declined"),
        "{declined:?}"
    );
    declining.finish().await;

    let accepting = serve(
        Arc::clone(&engine),
        Arc::clone(&audit),
        "writer",
        Confirming { proceed: true },
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await;
    let accepted = accepting.call("pg_delete", json!({"table": "items"})).await;
    assert_ne!(accepted.is_error, Some(true), "{accepted:?}");
    assert_eq!(structured(&accepted)["rows_affected"], 3);
    let data_dir = accepting.data_dir.clone();
    accepting.finish().await;

    let audit_file = std::fs::read_dir(&data_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .unwrap();
    let lines = std::fs::read_to_string(audit_file).unwrap();
    assert!(
        lines.contains("\"decision\":\"confirmed_argument\""),
        "{lines}"
    );
    assert!(
        lines.contains("\"decision\":\"confirmed_elicitation\""),
        "{lines}"
    );
    assert!(lines.contains("\"decision\":\"refused\""), "{lines}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_handles_commit_roll_back_and_refuse_other_principals() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let (engine, audit) = engine_for(&scratch, Mode::ReadWrite).await;
    let alice = serve(
        Arc::clone(&engine),
        Arc::clone(&audit),
        "alice",
        (),
        ClientLifecycleMode::Initialize,
    )
    .await;
    let begun = alice
        .call("pg_transaction", json!({"operation": "begin"}))
        .await;
    assert_ne!(begun.is_error, Some(true), "{begun:?}");
    let handle = structured(&begun)["handle"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let inside = alice
        .call(
            "pg_insert",
            json!({"table": "items", "rows": [{"id": 50, "name": "fifty"}], "transaction": handle}),
        )
        .await;
    assert_ne!(inside.is_error, Some(true), "{inside:?}");

    let invisible = alice
        .call(
            "pg_run_query",
            json!({"sql": "SELECT count(*) FROM items WHERE id = 50"}),
        )
        .await;
    assert_eq!(structured(&invisible)["rows"][0][0], "0");

    let without_handle = alice
        .call(
            "pg_insert",
            json!({"table": "items", "rows": [{"id": 51, "name": "fifty one"}]}),
        )
        .await;
    assert_eq!(structured(&without_handle)["code"], "handle.state");

    let saved = alice
        .call(
            "pg_transaction",
            json!({"operation": "savepoint", "handle": handle, "savepoint": "before_more"}),
        )
        .await;
    assert_eq!(structured(&saved)["handle"]["savepoints"][0], "before_more");
    alice
        .call(
            "pg_insert",
            json!({"table": "items", "rows": [{"id": 52, "name": "fifty two"}], "transaction": handle}),
        )
        .await;
    let back = alice
        .call(
            "pg_transaction",
            json!({"operation": "rollback_to", "handle": handle, "savepoint": "before_more"}),
        )
        .await;
    assert_ne!(back.is_error, Some(true), "{back:?}");

    let bob = serve(
        Arc::clone(&engine),
        Arc::clone(&audit),
        "bob",
        (),
        ClientLifecycleMode::Initialize,
    )
    .await;
    let stolen = bob
        .call(
            "pg_transaction",
            json!({"operation": "commit", "handle": handle}),
        )
        .await;
    assert_eq!(structured(&stolen)["code"], "handle.state");
    assert!(
        structured(&stolen)["message"]
            .as_str()
            .unwrap()
            .contains("another principal")
    );

    let committed = alice
        .call(
            "pg_transaction",
            json!({"operation": "commit", "handle": handle}),
        )
        .await;
    assert_eq!(structured(&committed)["handle"]["state"], "committed");
    let visible = alice
        .call(
            "pg_run_query",
            json!({"sql": "SELECT id FROM items WHERE id >= 50 ORDER BY id"}),
        )
        .await;
    let body = structured(&visible);
    assert_eq!(body["row_count"], 1);
    assert_eq!(body["rows"][0][0], "50");

    let again = alice
        .call(
            "pg_transaction",
            json!({"operation": "commit", "handle": handle}),
        )
        .await;
    let body = structured(&again);
    assert_eq!(body["code"], "handle.state");
    assert!(body["message"].as_str().unwrap().contains("committed"));

    let status = alice
        .call(
            "pg_transaction",
            json!({"operation": "status", "handle": handle}),
        )
        .await;
    assert_eq!(structured(&status)["handle"]["state"], "committed");

    let second = alice
        .call("pg_transaction", json!({"operation": "begin"}))
        .await;
    let second_handle = structured(&second)["handle"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    alice
        .call(
            "pg_delete",
            json!({"table": "items", "filter": "id = 50", "transaction": second_handle}),
        )
        .await;
    let rolled = alice
        .call(
            "pg_transaction",
            json!({"operation": "rollback", "handle": second_handle}),
        )
        .await;
    assert_eq!(structured(&rolled)["handle"]["state"], "rolled_back");
    let still_there = alice
        .call(
            "pg_run_query",
            json!({"sql": "SELECT count(*) FROM items WHERE id = 50"}),
        )
        .await;
    assert_eq!(structured(&still_there)["rows"][0][0], "1");
    alice.finish().await;
    bob.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_only_mode_refuses_reads_and_runs_writes() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let (engine, audit) = engine_for(&scratch, Mode::WriteOnly).await;
    let rig = serve(engine, audit, "writer", (), ClientLifecycleMode::Initialize).await;
    let refused = rig
        .call("pg_run_write", json!({"sql": "SELECT * FROM items"}))
        .await;
    assert_eq!(structured(&refused)["code"], "statement.refused");
    let inserted = rig
        .call(
            "pg_run_write",
            json!({"sql": "INSERT INTO items (id, name) VALUES (30, 'thirty') RETURNING id"}),
        )
        .await;
    assert_ne!(inserted.is_error, Some(true), "{inserted:?}");
    assert_eq!(structured(&inserted)["rows"][0][0], "30");
    let missing = rig
        .client
        .call_tool(CallToolRequestParams::new("pg_run_query"))
        .await;
    assert!(missing.is_err());
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unparsed_statements_run_once_through_parse_only_in_read_write_mode_with_confirm() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let sql = "UPDATE items SET qty = qty + 10 WHERE id = 1 RETURNING WITH (OLD AS o, NEW AS n) o.qty, n.qty";
    let (engine, audit) = engine_for(&scratch, Mode::ReadWrite).await;
    let rig = serve(engine, audit, "writer", (), ClientLifecycleMode::Initialize).await;
    let dry = rig
        .call("pg_run_write", json!({"sql": sql, "dry_run": true}))
        .await;
    assert_ne!(dry.is_error, Some(true), "{dry:?}");
    assert_eq!(structured(&dry)["kind"], "unparsed");
    assert_eq!(structured(&dry)["class"], "unknown");
    let unconfirmed = rig.call("pg_run_write", json!({"sql": sql})).await;
    assert_eq!(structured(&unconfirmed)["code"], "confirmation.required");
    let ran = rig
        .call("pg_run_write", json!({"sql": sql, "confirm": true}))
        .await;
    assert_ne!(ran.is_error, Some(true), "{ran:?}");
    assert_eq!(structured(&ran)["rows"][0][0], "1");
    assert_eq!(structured(&ran)["rows"][0][1], "11");
    let chained = rig
        .call(
            "pg_run_write",
            json!({"sql": format!("{sql}; DROP TABLE items"), "confirm": true}),
        )
        .await;
    assert_eq!(chained.is_error, Some(true), "{chained:?}");
    let still_there = rig
        .call("pg_run_query", json!({"sql": "SELECT count(*) FROM items"}))
        .await;
    assert_eq!(structured(&still_there)["rows"][0][0], "3");
    let data_dir = rig.data_dir.clone();
    rig.finish().await;
    let audit_file = std::fs::read_dir(&data_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .unwrap();
    let lines = std::fs::read_to_string(audit_file).unwrap();
    assert!(lines.contains("\"decision\":\"unparsed\""), "{lines}");
    assert!(lines.contains("\"operation\":\"unparsed\""), "{lines}");

    let (engine, audit) = engine_for(&scratch, Mode::WriteOnly).await;
    let rig = serve(engine, audit, "writer", (), ClientLifecycleMode::Initialize).await;
    let refused = rig
        .call("pg_run_write", json!({"sql": sql, "confirm": true}))
        .await;
    assert_eq!(structured(&refused)["code"], "statement.unparsable");
    rig.finish().await;
}
