use crate::support;

use std::sync::Arc;

use ownpg_core::audit::{Sink, Transport};
use ownpg_core::config::{FlagLayer, Mode, ToolGroup};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, stdio};
use rmcp::model::CallToolRequestParams;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::{Value, json};

fn parsed_host_and_port(dsn: &str) -> (String, u16) {
    let config: tokio_postgres::Config = dsn.parse().expect("a libpq connection string");
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => "127.0.0.1".to_owned(),
    };
    let port = config.get_ports().first().copied().unwrap_or(6432);
    (host, port)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_pool_status_reports_pools_and_stats_from_a_real_pgbouncer() {
    let Some(pgbouncer_dsn) = support::pgbouncer_admin_dsn() else {
        eprintln!(
            "{} is not set; the PgBouncer live test is skipped",
            support::PGBOUNCER_DSN_VARIABLE
        );
        return;
    };
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let (host, port) = parsed_host_and_port(&pgbouncer_dsn);
    let settings = Arc::new(scratch.settings_with(
        FlagLayer {
            host: Some(host),
            port: Some(port),
            mode: Some(Mode::ReadOnly),
            tools: Some(vec![ToolGroup::Monitoring]),
            ..FlagLayer::default()
        },
        &[("OWNPG_POOLED", "true")],
    ));
    let audit = Arc::new(Sink::disabled());
    let engine = Arc::new(
        Engine::start(Arc::clone(&settings), Hints::default())
            .await
            .expect("the engine starts through PgBouncer"),
    );
    let server = Arc::new(
        Server::new(
            Arc::clone(&engine),
            audit,
            Transport::Stdio,
            Principal::local(Some("pooler")),
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

    let listed = client.list_tools(None).await.unwrap();
    let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert!(names.contains(&"pg_pool_status"), "{names:?}");

    let call = |arguments: Value| {
        let client = &client;
        async move {
            client
                .call_tool(
                    CallToolRequestParams::new("pg_pool_status".to_owned())
                        .with_arguments(arguments.as_object().cloned().unwrap_or_default()),
                )
                .await
                .expect("the call returns a result")
        }
    };

    let pools = call(json!({})).await;
    assert_ne!(pools.is_error, Some(true), "{pools:?}");
    let structured = pools.structured_content.clone().unwrap();
    let rows = structured["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|row| row[0] == "pgbouncer" || row[0] == scratch.database),
        "{structured}"
    );

    let stats = call(json!({"command": "stats"})).await;
    assert_ne!(stats.is_error, Some(true), "{stats:?}");

    let version = call(json!({"command": "version"})).await;
    assert_ne!(version.is_error, Some(true), "{version:?}");
    let text = version.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("PgBouncer"), "{text}");

    drop(client);
    server_task
        .await
        .unwrap()
        .expect("the server stops cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_pool_status_fails_clearly_against_a_direct_postgresql_connection() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let settings = Arc::new(scratch.settings(FlagLayer {
        mode: Some(Mode::ReadOnly),
        tools: Some(vec![ToolGroup::Monitoring]),
        ..FlagLayer::default()
    }));
    let audit = Arc::new(Sink::disabled());
    let engine = Arc::new(
        Engine::start(Arc::clone(&settings), Hints::default())
            .await
            .expect("the engine starts"),
    );
    let server = Arc::new(
        Server::new(
            Arc::clone(&engine),
            audit,
            Transport::Stdio,
            Principal::local(Some("pooler")),
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

    let refused = client
        .call_tool(CallToolRequestParams::new("pg_pool_status".to_owned()))
        .await
        .expect("the call returns a result");
    assert_eq!(refused.is_error, Some(true), "{refused:?}");
    let structured = refused.structured_content.clone().unwrap();
    assert_eq!(structured["code"], "protocol.failed");

    drop(client);
    server_task
        .await
        .unwrap()
        .expect("the server stops cleanly");
}
