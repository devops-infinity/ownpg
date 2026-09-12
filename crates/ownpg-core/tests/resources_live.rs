#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ownpg_core::audit::{Sink, Transport};
use ownpg_core::config::{FlagLayer, Mode, ToolGroup};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, resources, stdio};
use rmcp::model::{
    CacheScope, CallToolRequestParams, ClientInfo, CompletionContext, ErrorCode,
    GetPromptRequestParams, ProtocolVersion, ReadResourceRequestParams,
    ResourceUpdatedNotificationParam, ServerNotification, SubscriptionFilter,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
struct Watching {
    updated: Arc<std::sync::Mutex<Vec<String>>>,
    list_changed: Arc<AtomicUsize>,
}

impl ClientHandler for Watching {
    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.updated.lock().unwrap().push(params.uri);
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.list_changed.fetch_add(1, Ordering::SeqCst);
    }

    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

struct Rig {
    client: RunningService<RoleClient, Watching>,
    handler: Watching,
    server_task: tokio::task::JoinHandle<ownpg_core::Result<ownpg_core::ExitClass>>,
    database: String,
}

async fn rig(scratch: &support::Scratch, lifecycle: ClientLifecycleMode) -> Rig {
    let settings = Arc::new(scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        mode: Some(Mode::ReadWrite),
        tools: Some(vec![ToolGroup::Ddl]),
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
            Principal::local(Some("resources")),
        )
        .await
        .expect("the server builds"),
    );
    let (client_side, server_side) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server_side);
    let server_task = tokio::spawn(stdio::serve(server, server_read, server_write));
    let (client_read, client_write) = tokio::io::split(client_side);
    let handler = Watching::default();
    let client = handler
        .clone()
        .serve_with_lifecycle((client_read, client_write), lifecycle)
        .await
        .expect("the client handshake completes");
    Rig {
        client,
        handler,
        server_task,
        database: scratch.database.clone(),
    }
}

impl Rig {
    async fn tool(&self, tool: &str, arguments: Value) -> Value {
        let object = arguments.as_object().cloned().unwrap_or_default();
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(object))
            .await
            .expect("the call returns a result");
        assert_ne!(result.is_error, Some(true), "{tool}: {result:?}");
        result.structured_content.unwrap_or(Value::Null)
    }

    fn schema_uri(&self) -> String {
        resources::schema_uri(&self.database, "app")
    }

    fn table_uri(&self, table: &str) -> String {
        resources::table_uri(&self.database, "app", table)
    }

    async fn read(&self, uri: &str) -> Value {
        let result = self
            .client
            .read_resource(ReadResourceRequestParams::new(uri.to_owned()))
            .await
            .expect("the resource reads");
        assert_eq!(result.ttl_ms, Some(60_000));
        assert_eq!(result.cache_scope, Some(CacheScope::Private));
        let rmcp::model::ResourceContents::TextResourceContents {
            text, mime_type, ..
        } = &result.contents[0]
        else {
            panic!("a text resource is expected");
        };
        assert_eq!(mime_type.as_deref(), Some("application/json"));
        serde_json::from_str(text).unwrap()
    }

    async fn finish(self) {
        drop(self.client);
        self.server_task
            .await
            .unwrap()
            .expect("the server stops cleanly");
    }
}

async fn prepare(scratch: &support::Scratch) {
    let client = scratch.client().await;
    client
        .batch_execute(
            "CREATE SCHEMA app; \
             CREATE TABLE app.orders (id bigint primary key, customer_id bigint not null, total numeric(12,2)); \
             CREATE TABLE app.customers (id bigint primary key, name text); \
             CREATE VIEW app.order_totals AS SELECT customer_id, sum(total) AS total FROM app.orders GROUP BY customer_id; \
             COMMENT ON TABLE app.orders IS 'one row per order';",
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resources_mirror_the_catalog_tools_and_refuse_foreign_uris() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let rig = rig(&scratch, ClientLifecycleMode::Initialize).await;

    let templates = rig.client.list_resource_templates(None).await.unwrap();
    let uris: Vec<&str> = templates
        .resource_templates
        .iter()
        .map(|template| template.uri_template.as_str())
        .collect();
    assert_eq!(
        uris,
        [
            "postgres://{database}/{schema}",
            "postgres://{database}/{schema}/{table}"
        ]
    );
    assert_eq!(templates.ttl_ms, Some(60_000));
    assert_eq!(templates.cache_scope, Some(CacheScope::Private));

    let listed = rig.client.list_resources(None).await.unwrap();
    let names: Vec<&str> = listed
        .resources
        .iter()
        .map(|resource| resource.name.as_str())
        .collect();
    assert_eq!(names, ["app", "customers", "order_totals", "orders"]);
    assert_eq!(listed.resources[0].uri, rig.schema_uri());
    assert_eq!(listed.resources[3].uri, rig.table_uri("orders"));
    assert_eq!(
        listed.resources[3].description.as_deref(),
        Some("one row per order")
    );
    assert_eq!(listed.ttl_ms, Some(60_000));

    let described = rig
        .tool(
            "pg_describe",
            json!({"name": "orders", "object_type": "relation"}),
        )
        .await;
    let read = rig.read(&rig.table_uri("orders")).await;
    assert_eq!(read, described);

    let schema = rig.read(&rig.schema_uri()).await;
    let objects: Vec<&str> = schema["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| object["name"].as_str().unwrap())
        .collect();
    assert!(objects.contains(&"orders"), "{objects:?}");
    assert!(objects.contains(&"order_totals"), "{objects:?}");

    for uri in [
        "postgres://other/app/orders".to_owned(),
        format!("postgres://{}/other/orders", rig.database),
        rig.table_uri("missing"),
        "https://example.com/x".to_owned(),
    ] {
        let error = rig
            .client
            .read_resource(ReadResourceRequestParams::new(uri.clone()))
            .await
            .unwrap_err();
        let rmcp::service::ServiceError::McpError(data) = error else {
            panic!("{uri}: an MCP error is expected, got {error:?}");
        };
        assert_eq!(data.code, ErrorCode::RESOURCE_NOT_FOUND, "{uri}: {data:?}");
    }
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompts_render_with_arguments_and_complete_names() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let rig = rig(&scratch, ClientLifecycleMode::Initialize).await;

    let listed = rig.client.list_prompts(None).await.unwrap();
    let names: Vec<&str> = listed
        .prompts
        .iter()
        .map(|prompt| prompt.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "diagnose_slow_query",
            "review_indexes",
            "plan_column_change"
        ]
    );
    assert_eq!(listed.ttl_ms, Some(60_000));

    let mut request = GetPromptRequestParams::new("review_indexes");
    request.arguments = Some(json!({"table": "orders"}).as_object().cloned().unwrap());
    let prompt = rig.client.get_prompt(request).await.unwrap();
    assert_eq!(prompt.messages.len(), 1);
    let rendered = serde_json::to_string(&prompt.messages[0]).unwrap();
    assert!(rendered.contains("pg_describe on orders"), "{rendered}");
    assert!(rendered.contains("app"), "{rendered}");
    assert!(!rendered.contains("pg_indexes_health"), "{rendered}");

    let mut request = GetPromptRequestParams::new("diagnose_slow_query");
    request.arguments = Some(
        json!({"sql": "SELECT * FROM orders WHERE customer_id = 7", "goal": "under 5 ms"})
            .as_object()
            .cloned()
            .unwrap(),
    );
    let prompt = rig.client.get_prompt(request).await.unwrap();
    let rendered = serde_json::to_string(&prompt.messages[0]).unwrap();
    assert!(rendered.contains("under 5 ms"), "{rendered}");
    assert!(rendered.contains("customer_id = 7"), "{rendered}");

    let mut request = GetPromptRequestParams::new("plan_column_change");
    request.arguments = Some(json!({"table": "orders"}).as_object().cloned().unwrap());
    let error = rig.client.get_prompt(request).await.unwrap_err();
    let rmcp::service::ServiceError::McpError(data) = error else {
        panic!("an MCP error is expected, got {error:?}");
    };
    assert_eq!(data.code, ErrorCode::INVALID_PARAMS);
    assert!(data.message.contains("column"), "{data:?}");

    let tables = rig
        .client
        .complete_prompt_simple("plan_column_change", "table", "ord")
        .await
        .unwrap();
    assert_eq!(tables, ["order_totals", "orders"]);
    let columns = rig
        .client
        .complete_prompt_argument(
            "plan_column_change",
            "column",
            "cu",
            Some(CompletionContext::with_arguments(
                [("table".to_owned(), "orders".to_owned())]
                    .into_iter()
                    .collect(),
            )),
        )
        .await
        .unwrap();
    assert_eq!(columns.values, ["customer_id"]);
    assert_eq!(columns.total, Some(1));
    let all = rig
        .client
        .complete_resource_simple("postgres://{database}/{schema}/{table}", "table", "")
        .await
        .unwrap();
    assert_eq!(all, ["customers", "order_totals", "orders"]);
    let database = rig
        .client
        .complete_resource_simple("postgres://{database}/{schema}/{table}", "database", "")
        .await
        .unwrap();
    assert_eq!(database, std::slice::from_ref(&rig.database));
    let none = rig
        .client
        .complete_prompt_simple("review_indexes", "table", "zzz")
        .await
        .unwrap();
    assert!(none.is_empty());
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ddl_call_announces_the_table_and_schema_to_a_legacy_client() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let rig = rig(&scratch, ClientLifecycleMode::Initialize).await;
    rig.tool(
        "pg_column",
        json!({"operation": "add", "table": "orders", "column": "note", "data_type": "text", "dry_run": true}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rig.handler.updated.lock().unwrap().is_empty());

    rig.tool(
        "pg_column",
        json!({"operation": "add", "table": "orders", "column": "note", "data_type": "text"}),
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while rig.handler.list_changed.load(Ordering::SeqCst) == 0
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        *rig.handler.updated.lock().unwrap(),
        vec![rig.table_uri("orders")]
    );
    assert_eq!(rig.handler.list_changed.load(Ordering::SeqCst), 1);
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ddl_call_reaches_a_subscription_stream_on_the_current_protocol() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    prepare(&scratch).await;
    let rig = rig(
        &scratch,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        },
    )
    .await;
    let orders = rig.table_uri("orders");
    let mut subscription = rig
        .client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .resource_subscription(orders.clone())
                .build(),
        )
        .await
        .expect("the subscription is acknowledged");
    assert_eq!(
        subscription.acknowledged().resources_list_changed,
        Some(true)
    );
    assert_eq!(
        subscription.acknowledged().resource_subscriptions,
        Some(vec![orders.clone()])
    );

    rig.tool(
        "pg_index",
        json!({"operation": "create", "table": "orders", "name": "orders_customer_idx", "columns": ["customer_id"]}),
    )
    .await;
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let next = tokio::time::timeout(Duration::from_secs(5), subscription.next())
            .await
            .expect("a notification arrives")
            .unwrap()
            .expect("the stream stays open");
        match next {
            ServerNotification::ResourceUpdatedNotification(update) => {
                seen.push(format!("updated {}", update.params.uri));
            }
            ServerNotification::ResourceListChangedNotification(_) => {
                seen.push("list_changed".to_owned());
            }
            other => panic!("unexpected notification {other:?}"),
        }
    }
    assert_eq!(
        seen,
        [format!("updated {orders}"), "list_changed".to_owned()]
    );
    assert!(rig.handler.updated.lock().unwrap().is_empty());
    subscription.cancel().await.unwrap();
    rig.finish().await;
}
