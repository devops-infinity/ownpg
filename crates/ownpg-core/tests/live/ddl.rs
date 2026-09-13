use crate::support;

use std::sync::Arc;

use ownpg_core::audit::{Sink, Transport};
use ownpg_core::config::{FlagLayer, Mode, ToolGroup};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, stdio};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::{ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::{Value, json};

struct Rig {
    client: RunningService<RoleClient, ()>,
    server_task: tokio::task::JoinHandle<ownpg_core::Result<ownpg_core::ExitClass>>,
}

async fn rig(scratch: &support::Scratch) -> Rig {
    let client = scratch.client().await;
    client
        .batch_execute("CREATE SCHEMA app; CREATE SCHEMA other;")
        .await
        .unwrap();
    let settings = Arc::new(scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        mode: Some(Mode::ReadWrite),
        tools: Some(vec![ToolGroup::Ddl, ToolGroup::Roles]),
        ..FlagLayer::default()
    }));
    let audit = Arc::new(Sink::disabled());
    let engine = Engine::start(Arc::clone(&settings), Hints::default())
        .await
        .expect("the engine starts");
    let server = Arc::new(
        Server::new(
            Arc::new(engine),
            audit,
            Transport::Stdio,
            Principal::local(Some("ddl")),
        )
        .await
        .expect("the server builds"),
    );
    let (client_side, server_side) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server_task = tokio::spawn(stdio::serve(server, server_read, server_write));
    let (client_read, client_write) = tokio::io::split(client_side);
    let client =
        ().serve_with_lifecycle((client_read, client_write), ClientLifecycleMode::Initialize)
            .await
            .expect("the client handshake completes");
    Rig {
        client,
        server_task,
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

    async fn dry_run_sql(&self, tool: &str, mut arguments: Value) -> String {
        arguments["dry_run"] = json!(true);
        let body = self.ok(tool, arguments).await;
        body["sql"].as_str().unwrap().to_owned()
    }

    async fn failed(&self, tool: &str, arguments: Value) -> Value {
        let result = self.call(tool, arguments).await;
        assert_eq!(result.is_error, Some(true), "{tool}: {result:?}");
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
async fn the_ddl_tools_build_a_schema_end_to_end() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let rig = rig(&scratch).await;
    let listed = rig.client.list_tools(None).await.unwrap();
    let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    for expected in [
        "pg_table",
        "pg_column",
        "pg_constraint",
        "pg_index",
        "pg_view",
        "pg_sequence",
        "pg_routine",
        "pg_trigger",
        "pg_type",
        "pg_extension",
        "pg_comment",
        "pg_role",
        "pg_grant",
        "pg_policy",
        "pg_privileges",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }

    let create = rig
        .dry_run_sql(
            "pg_table",
            json!({
                "operation": "create", "name": "customers",
                "columns": [
                    {"name": "id", "data_type": "bigint", "identity": "always", "primary_key": true},
                    {"name": "email", "data_type": "text", "not_null": true, "unique": true},
                    {"name": "created_at", "data_type": "timestamptz", "not_null": true, "default": "now()"},
                    {"name": "score", "data_type": "integer", "check": "score >= 0"}
                ]
            }),
        )
        .await;
    insta::assert_snapshot!("create_customers", create);
    rig.ok(
        "pg_table",
        json!({
            "operation": "create", "name": "customers",
            "columns": [
                {"name": "id", "data_type": "bigint", "identity": "always", "primary_key": true},
                {"name": "email", "data_type": "text", "not_null": true, "unique": true},
                {"name": "created_at", "data_type": "timestamptz", "not_null": true, "default": "now()"},
                {"name": "score", "data_type": "integer", "check": "score >= 0"}
            ]
        }),
    )
    .await;
    rig.ok(
        "pg_table",
        json!({
            "operation": "create", "name": "orders",
            "columns": [
                {"name": "id", "data_type": "bigint", "identity": "by_default", "primary_key": true},
                {"name": "customer_id", "data_type": "bigint", "not_null": true, "references": "customers", "references_column": "id", "on_delete": "cascade"},
                {"name": "total", "data_type": "numeric(12, 2)", "not_null": true, "default": "0"},
                {"name": "status", "data_type": "text", "not_null": true, "default": "'new'"}
            ]
        }),
    )
    .await;

    let outside = rig
        .failed(
            "pg_table",
            json!({"operation": "create", "name": "other.leak", "columns": [{"name": "id", "data_type": "int"}]}),
        )
        .await;
    assert_eq!(outside["code"], "statement.refused");

    let smuggled = rig
        .failed(
            "pg_column",
            json!({"operation": "add", "table": "orders", "column": "note", "data_type": "text); DROP TABLE customers; --"}),
        )
        .await;
    assert_eq!(smuggled["code"], "argument.invalid");

    rig.ok(
        "pg_column",
        json!({"operation": "add", "table": "orders", "column": "note", "data_type": "text"}),
    )
    .await;
    rig.ok(
        "pg_column",
        json!({"operation": "set_default", "table": "orders", "column": "note", "default": "'none'"}),
    )
    .await;
    let needs_confirm = rig
        .failed(
            "pg_column",
            json!({"operation": "alter_type", "table": "orders", "column": "total", "data_type": "numeric(14, 2)", "using": "total::numeric(14, 2)"}),
        )
        .await;
    assert_eq!(needs_confirm["code"], "confirmation.required");
    rig.ok(
        "pg_column",
        json!({"operation": "alter_type", "table": "orders", "column": "total", "data_type": "numeric(14, 2)", "using": "total::numeric(14, 2)", "confirm": true}),
    )
    .await;
    let missing = rig
        .failed(
            "pg_column",
            json!({"operation": "alter_type", "table": "orders", "column": "total"}),
        )
        .await;
    assert!(missing["message"].as_str().unwrap().contains("data_type"));

    rig.ok(
        "pg_constraint",
        json!({"operation": "add", "table": "orders", "name": "orders_total_positive", "kind": "check", "expression": "total >= 0", "not_valid": true}),
    )
    .await;
    rig.ok(
        "pg_constraint",
        json!({"operation": "validate", "table": "orders", "name": "orders_total_positive"}),
    )
    .await;
    rig.ok(
        "pg_constraint",
        json!({"operation": "add", "table": "orders", "name": "orders_note_unique", "kind": "unique", "columns": ["note"], "nulls_not_distinct": true}),
    )
    .await;
    let drop_constraint = rig
        .dry_run_sql(
            "pg_constraint",
            json!({"operation": "drop", "table": "orders", "name": "orders_note_unique", "confirm": true}),
        )
        .await;
    assert_eq!(
        drop_constraint,
        "ALTER TABLE \"app\".\"orders\" DROP CONSTRAINT \"orders_note_unique\""
    );

    rig.ok(
        "pg_index",
        json!({"operation": "create", "name": "orders_status_idx", "table": "orders", "columns": ["status", "id DESC"], "where_clause": "status <> 'done'"}),
    )
    .await;
    let begun = rig
        .call("pg_transaction", json!({"operation": "begin"}))
        .await;
    let handle = begun.structured_content.unwrap()["handle"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let refused = rig
        .failed(
            "pg_index",
            json!({"operation": "create", "name": "orders_total_idx", "table": "orders", "columns": ["total"], "concurrently": true, "transaction": handle}),
        )
        .await;
    assert_eq!(refused["code"], "statement.refused");
    rig.ok(
        "pg_transaction",
        json!({"operation": "rollback", "handle": handle}),
    )
    .await;
    rig.ok(
        "pg_index",
        json!({"operation": "create", "name": "orders_total_idx", "table": "orders", "columns": ["total"], "concurrently": true}),
    )
    .await;
    rig.ok(
        "pg_index",
        json!({"operation": "reindex", "name": "orders_total_idx"}),
    )
    .await;

    rig.ok(
        "pg_view",
        json!({"operation": "create", "name": "open_orders", "query": "SELECT id, total FROM orders WHERE status = 'new'", "security_invoker": true}),
    )
    .await;
    rig.ok(
        "pg_view",
        json!({"operation": "create", "name": "order_totals", "materialized": true, "query": "SELECT status, sum(total) AS total FROM orders GROUP BY status", "with_data": false}),
    )
    .await;
    rig.ok(
        "pg_view",
        json!({"operation": "refresh", "name": "order_totals", "materialized": true}),
    )
    .await;
    let bad_view = rig
        .failed(
            "pg_view",
            json!({"operation": "create", "name": "leak", "query": "DELETE FROM orders"}),
        )
        .await;
    assert_eq!(bad_view["code"], "statement.refused");

    rig.ok(
        "pg_sequence",
        json!({"operation": "create", "name": "ticket_seq", "start": "100", "increment": "5", "cycle": "off"}),
    )
    .await;
    rig.ok(
        "pg_sequence",
        json!({"operation": "alter", "name": "ticket_seq", "restart": "500"}),
    )
    .await;
    let bad_number = rig
        .failed(
            "pg_sequence",
            json!({"operation": "alter", "name": "ticket_seq", "restart": "five"}),
        )
        .await;
    assert_eq!(bad_number["code"], "argument.invalid");

    let routine_sql = rig
        .dry_run_sql(
            "pg_routine",
            json!({
                "operation": "create", "name": "order_total", "or_replace": true,
                "arguments": [{"name": "order_id", "data_type": "bigint"}],
                "returns": "numeric", "language": "sql", "volatility": "stable", "strict": "on",
                "security_definer": "on",
                "body": "SELECT total FROM app.orders WHERE id = order_id"
            }),
        )
        .await;
    insta::assert_snapshot!("create_order_total", routine_sql);
    rig.ok(
        "pg_routine",
        json!({
            "operation": "create", "name": "order_total", "or_replace": true,
            "arguments": [{"name": "order_id", "data_type": "bigint"}],
            "returns": "numeric", "language": "sql", "volatility": "stable", "strict": "on",
            "security_definer": "on",
            "body": "SELECT total FROM app.orders WHERE id = order_id"
        }),
    )
    .await;
    rig.ok(
        "pg_routine",
        json!({
            "operation": "create", "name": "touch_note", "returns": "trigger", "language": "plpgsql",
            "body": "BEGIN NEW.note := 'touched'; RETURN NEW; END"
        }),
    )
    .await;
    rig.ok(
        "pg_trigger",
        json!({"operation": "create", "name": "orders_touch", "table": "orders", "timing": "before", "events": ["insert", "update"], "for_each_row": true, "function": "touch_note"}),
    )
    .await;
    rig.ok(
        "pg_trigger",
        json!({"operation": "disable", "name": "orders_touch", "table": "orders"}),
    )
    .await;
    rig.ok(
        "pg_trigger",
        json!({"operation": "enable", "name": "orders_touch", "table": "orders"}),
    )
    .await;

    rig.ok(
        "pg_type",
        json!({"operation": "create_enum", "name": "order_state", "labels": ["new", "paid", "shipped"]}),
    )
    .await;
    rig.ok(
        "pg_type",
        json!({"operation": "add_value", "name": "order_state", "value": "cancelled", "after": "paid"}),
    )
    .await;
    rig.ok(
        "pg_type",
        json!({"operation": "rename_value", "name": "order_state", "value": "cancelled", "new_value": "canceled"}),
    )
    .await;
    rig.ok(
        "pg_type",
        json!({"operation": "create_domain", "name": "positive_money", "base_type": "numeric", "check": "VALUE >= 0", "constraint_name": "positive_money_check"}),
    )
    .await;
    rig.ok(
        "pg_type",
        json!({"operation": "create_composite", "name": "address", "attributes": [{"name": "street", "data_type": "text"}, {"name": "city", "data_type": "text"}]}),
    )
    .await;
    rig.ok(
        "pg_comment",
        json!({"target": "table", "name": "orders", "comment": "orders placed by customers"}),
    )
    .await;
    rig.ok(
        "pg_comment",
        json!({"target": "column", "name": "orders", "member": "total", "comment": "gross total"}),
    )
    .await;
    let available = rig
        .ok("pg_extension", json!({"operation": "list_available"}))
        .await;
    assert!(available["row_count"].as_u64().unwrap() > 0);
    let insert_row = rig
        .ok(
            "pg_run_write",
            json!({"sql": "INSERT INTO customers (email, score) VALUES ('a@example.com', 5) RETURNING id"}),
        )
        .await;
    assert_eq!(insert_row["rows"][0][0], "1");
    rig.ok(
        "pg_insert",
        json!({"table": "orders", "rows": [{"customer_id": 1, "total": 12.5}]}),
    )
    .await;
    let described = rig.ok("pg_describe", json!({"name": "orders"})).await;
    let relation = &described["relation"];
    assert_eq!(relation["comment"], "orders placed by customers");
    assert!(relation["triggers"].as_array().unwrap().len() == 1);
    assert!(relation["indexes"].as_array().unwrap().len() >= 3);
    let touched = rig
        .ok(
            "pg_run_query",
            json!({"sql": "SELECT note, order_total(id) FROM orders"}),
        )
        .await;
    assert_eq!(touched["rows"][0][0], "touched");
    assert_eq!(touched["rows"][0][1], "12.50");

    let role_sql = rig
        .dry_run_sql(
            "pg_role",
            json!({"operation": "create", "name": "app_reader", "can_login": "on", "password": "s3cret", "connection_limit": "5"}),
        )
        .await;
    assert_eq!(
        role_sql,
        "CREATE ROLE \"app_reader\" WITH LOGIN CONNECTION LIMIT 5 PASSWORD 's3cret'"
    );
    let refused_role = rig
        .failed(
            "pg_role",
            json!({"operation": "create", "name": "app_reader", "can_login": "on"}),
        )
        .await;
    assert_eq!(refused_role["sqlstate"], "42501");
    let template = rig
        .dry_run_sql(
            "pg_privileges",
            json!({"operation": "apply_template", "name": "app_reader", "template": "read_only"}),
        )
        .await;
    insta::assert_snapshot!("read_only_template", template);
    let grant_sql = rig
        .dry_run_sql(
            "pg_grant",
            json!({"operation": "grant", "target": "all_tables", "privileges": ["select"], "roles": ["public"]}),
        )
        .await;
    assert_eq!(
        grant_sql,
        "GRANT SELECT ON ALL TABLES IN SCHEMA \"app\" TO PUBLIC"
    );
    rig.ok(
        "pg_grant",
        json!({"operation": "grant", "target": "table", "names": ["orders"], "privileges": ["select"], "roles": ["public"]}),
    )
    .await;
    rig.ok(
        "pg_policy",
        json!({"operation": "create", "name": "orders_own", "table": "orders", "command": "select", "using": "customer_id = 1"}),
    )
    .await;
    rig.ok(
        "pg_policy",
        json!({"operation": "enable_rls", "table": "orders"}),
    )
    .await;
    let privileges = rig
        .ok(
            "pg_privileges",
            json!({"operation": "list_object", "name": "orders"}),
        )
        .await;
    assert!(
        privileges["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["grantee"] == "PUBLIC" && row["privilege"] == "SELECT")
    );
    let mine = rig
        .ok(
            "pg_privileges",
            json!({"operation": "list_role", "name": scratch.user}),
        )
        .await;
    assert_eq!(mine["has_schema_usage"], true);
    assert!(!mine["tables"].as_array().unwrap().is_empty());

    let drop_needs_confirm = rig
        .failed(
            "pg_table",
            json!({"operation": "drop", "name": "customers", "cascade": true}),
        )
        .await;
    assert_eq!(drop_needs_confirm["code"], "confirmation.required");
    rig.ok(
        "pg_table",
        json!({"operation": "drop", "name": "customers", "cascade": true, "confirm": true}),
    )
    .await;
    let gone = rig
        .failed("pg_describe", json!({"name": "customers"}))
        .await;
    assert_eq!(gone["code"], "argument.invalid");
    rig.finish().await;
}
