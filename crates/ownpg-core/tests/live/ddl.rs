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
        "pg_publication",
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

    let described_index = rig
        .ok("pg_describe", json!({"name": "orders_status_idx"}))
        .await;
    assert_eq!(described_index["kind"], "index");
    assert_eq!(
        described_index["relation"]["kind"], "index",
        "{described_index}"
    );

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
            "body": "SELECT total FROM app.orders WHERE id = order_id",
            "confirm": true
        }),
    )
    .await;
    rig.ok(
        "pg_routine",
        json!({
            "operation": "create", "name": "touch_note", "returns": "trigger", "language": "plpgsql",
            "body": "BEGIN NEW.note := 'touched'; RETURN NEW; END",
            "confirm": true
        }),
    )
    .await;
    rig.ok(
        "pg_routine",
        json!({
            "operation": "alter", "name": "touch_note", "security_definer": "on",
            "confirm": true
        }),
    )
    .await;
    let pinned: Vec<String> = scratch
        .client()
        .await
        .query_one(
            "SELECT coalesce(proconfig, '{}') FROM pg_proc WHERE proname = 'touch_note'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(pinned, ["search_path=app, pg_temp"]);
    let shadowed = rig
        .failed(
            "pg_routine",
            json!({
                "operation": "alter", "name": "touch_note", "security_definer": "on",
                "set_config": ["search_path = public"], "confirm": true
            }),
        )
        .await;
    assert_eq!(shadowed["code"], "argument.invalid");
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
        "CREATE ROLE \"app_reader\" WITH LOGIN CONNECTION LIMIT 5 PASSWORD '***'"
    );
    assert!(!role_sql.contains("s3cret"), "{role_sql}");
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

    let create_sql = rig
        .dry_run_sql(
            "pg_publication",
            json!({"operation": "create", "name": "orders_pub", "tables": ["orders"], "publish": "insert, update"}),
        )
        .await;
    assert!(create_sql.starts_with("CREATE PUBLICATION"), "{create_sql}");
    assert!(create_sql.contains("insert, update"), "{create_sql}");
    rig.ok(
        "pg_publication",
        json!({"operation": "create", "name": "orders_pub", "tables": ["orders"], "publish": "insert, update"}),
    )
    .await;
    let seen = rig
        .ok(
            "pg_run_query",
            json!({"sql": "SELECT pubname, puballtables FROM pg_publication WHERE pubname = 'orders_pub'"}),
        )
        .await;
    assert_eq!(seen["rows"][0][0], "orders_pub");
    assert_eq!(seen["rows"][0][1], false);
    let both_set = rig
        .failed(
            "pg_publication",
            json!({"operation": "create", "name": "bad_pub", "for_all_tables": true, "tables": ["orders"]}),
        )
        .await;
    assert_eq!(both_set["code"], "argument.invalid");

    let add_sql = rig
        .dry_run_sql(
            "pg_publication",
            json!({"operation": "add_tables", "name": "orders_pub", "tables": ["customers"]}),
        )
        .await;
    assert_eq!(
        add_sql,
        "ALTER PUBLICATION \"orders_pub\" ADD TABLE \"app\".\"customers\""
    );
    rig.ok(
        "pg_publication",
        json!({"operation": "add_tables", "name": "orders_pub", "tables": ["customers"]}),
    )
    .await;
    let two_tables = rig
        .ok(
            "pg_run_query",
            json!({"sql": "SELECT count(*) FROM pg_publication_tables WHERE pubname = 'orders_pub'"}),
        )
        .await;
    assert_eq!(two_tables["rows"][0][0], 2);

    let drop_tables_needs_confirm = rig
        .failed(
            "pg_publication",
            json!({"operation": "drop_tables", "name": "orders_pub", "tables": ["customers"]}),
        )
        .await;
    assert_eq!(drop_tables_needs_confirm["code"], "confirmation.required");
    rig.ok(
        "pg_publication",
        json!({"operation": "drop_tables", "name": "orders_pub", "tables": ["customers"], "confirm": true}),
    )
    .await;
    let back_to_one = rig
        .ok(
            "pg_run_query",
            json!({"sql": "SELECT count(*) FROM pg_publication_tables WHERE pubname = 'orders_pub'"}),
        )
        .await;
    assert_eq!(back_to_one["rows"][0][0], 1);

    rig.ok(
        "pg_publication",
        json!({"operation": "add_tables", "name": "orders_pub", "tables": ["customers"]}),
    )
    .await;
    let set_tables_needs_confirm = rig
        .failed(
            "pg_publication",
            json!({"operation": "set_tables", "name": "orders_pub", "tables": ["orders"]}),
        )
        .await;
    assert_eq!(set_tables_needs_confirm["code"], "confirmation.required");
    rig.ok(
        "pg_publication",
        json!({"operation": "set_tables", "name": "orders_pub", "tables": ["orders"], "confirm": true}),
    )
    .await;
    let replaced = rig
        .ok(
            "pg_run_query",
            json!({"sql": "SELECT tablename FROM pg_publication_tables WHERE pubname = 'orders_pub'"}),
        )
        .await;
    assert_eq!(replaced["row_count"], 1);
    assert_eq!(replaced["rows"][0][0], "orders");

    rig.ok(
        "pg_publication",
        json!({"operation": "rename", "name": "orders_pub", "new_name": "orders_pub_v2"}),
    )
    .await;
    rig.ok(
        "pg_publication",
        json!({"operation": "set_owner", "name": "orders_pub_v2", "owner": scratch.user.clone()}),
    )
    .await;
    let unconfirmed = rig
        .failed(
            "pg_publication",
            json!({"operation": "create", "name": "everything_pub", "for_all_tables": true}),
        )
        .await;
    assert_eq!(unconfirmed["code"], "confirmation.required");
    let all_tables = rig
        .failed(
            "pg_publication",
            json!({"operation": "create", "name": "everything_pub", "for_all_tables": true, "confirm": true}),
        )
        .await;
    assert_eq!(all_tables["code"], "sql.failed");
    assert_eq!(all_tables["sqlstate"], "42501");
    let drop_needs_confirm_pub = rig
        .failed(
            "pg_publication",
            json!({"operation": "drop", "name": "orders_pub_v2"}),
        )
        .await;
    assert_eq!(drop_needs_confirm_pub["code"], "confirmation.required");
    rig.ok(
        "pg_publication",
        json!({"operation": "drop", "name": "orders_pub_v2", "confirm": true}),
    )
    .await;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cascade_lists_its_dependents_and_refuses_to_reach_outside_the_schema() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let rig = rig(&scratch).await;
    scratch
        .client()
        .await
        .batch_execute(
            "CREATE TABLE app.t (id int, note text); \
             CREATE VIEW app.v AS SELECT id FROM app.t; \
             CREATE VIEW app.vv AS SELECT id FROM app.v; \
             CREATE VIEW other.w AS SELECT note FROM app.t;",
        )
        .await
        .unwrap();

    let refused = rig
        .failed(
            "pg_table",
            json!({"operation": "drop", "name": "t", "cascade": true, "dry_run": true}),
        )
        .await;
    assert_eq!(refused["code"], "statement.refused", "{refused}");
    let rule = refused["rule"].as_str().unwrap();
    assert!(rule.contains("view other.w"), "{rule}");
    assert!(!rule.contains("app.v"), "{rule}");
    let raw = rig
        .failed(
            "pg_run_write",
            json!({"sql": "DROP TABLE app.t CASCADE", "confirm": true}),
        )
        .await;
    assert_eq!(raw["code"], "statement.refused", "{raw}");
    assert!(
        raw["rule"]
            .as_str()
            .unwrap()
            .contains("belongs to the ddl tools"),
        "{raw}"
    );
    let column = rig
        .failed(
            "pg_column",
            json!({"operation": "drop", "table": "t", "column": "note", "cascade": true, "confirm": true}),
        )
        .await;
    assert!(
        column["rule"].as_str().unwrap().contains("other.w"),
        "{column}"
    );
    rig.ok(
        "pg_column",
        json!({"operation": "drop", "table": "t", "column": "id", "cascade": true, "dry_run": true}),
    )
    .await;

    scratch
        .client()
        .await
        .batch_execute("DROP VIEW other.w")
        .await
        .unwrap();
    let dry = rig
        .ok(
            "pg_table",
            json!({"operation": "drop", "name": "t", "cascade": true, "dry_run": true}),
        )
        .await;
    let listed: Vec<&str> = dry["cascades_to"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect();
    assert_eq!(listed, ["view app.v", "view app.vv"], "{dry}");
    rig.ok(
        "pg_table",
        json!({"operation": "drop", "name": "t", "cascade": true, "confirm": true}),
    )
    .await;
    let remaining: i64 = scratch
        .client()
        .await
        .query_one(
            "SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'app'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(remaining, 0);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_temporary_table_is_created_without_the_schema_prefix() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let rig = rig(&scratch).await;
    let sql = rig
        .dry_run_sql(
            "pg_table",
            json!({"operation": "create", "name": "scratch_rows", "temporary": true, "columns": [{"name": "id", "data_type": "integer"}]}),
        )
        .await;
    assert_eq!(
        sql,
        "CREATE TEMPORARY TABLE \"scratch_rows\" (\"id\" integer)"
    );
    rig.ok(
        "pg_table",
        json!({"operation": "create", "name": "scratch_rows", "temporary": true, "columns": [{"name": "id", "data_type": "integer"}]}),
    )
    .await;
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trailing_comment_in_a_view_query_never_hides_its_options() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let rig = rig(&scratch).await;
    scratch
        .client()
        .await
        .batch_execute("CREATE TABLE app.t (id int)")
        .await
        .unwrap();
    rig.ok(
        "pg_view",
        json!({"operation": "create", "name": "checked", "query": "SELECT id FROM app.t WHERE id > 0 -- positive only", "check_option": "local"}),
    )
    .await;
    rig.ok(
        "pg_view",
        json!({"operation": "create", "name": "empty_mv", "materialized": true, "with_data": false, "query": "SELECT id FROM app.t -- none yet"}),
    )
    .await;
    let client = scratch.client().await;
    let option: String = client
        .query_one(
            "SELECT check_option::text FROM information_schema.views WHERE table_schema = 'app' AND table_name = 'checked'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(option, "LOCAL");
    let populated: bool = client
        .query_one(
            "SELECT ispopulated FROM pg_catalog.pg_matviews WHERE schemaname = 'app' AND matviewname = 'empty_mv'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!populated);
    let unnamed = rig
        .failed(
            "pg_index",
            json!({"operation": "create", "table": "t", "columns": ["id"], "if_not_exists": true}),
        )
        .await;
    assert_eq!(unnamed["code"], "argument.invalid");
    rig.finish().await;
}
