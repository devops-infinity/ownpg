#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing
)]

mod support;

use ownpg_core::config::FlagLayer;
use ownpg_core::connect::role::RoleProfile;
use ownpg_core::connect::{Connector, Via};

#[tokio::test]
async fn a_tcp_connection_pins_the_schema_reads_the_version_and_the_role() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let settings = scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        ..FlagLayer::default()
    });
    let connector = Connector::new(support::shared(settings));
    let session = connector
        .connect()
        .await
        .expect("the scratch database connects");
    assert_eq!(session.info.via, Via::Tcp);
    assert_eq!(session.info.database, scratch.database);
    assert_eq!(session.info.search_path, "app");
    assert!(session.info.server_version_num >= 140_000);
    assert_eq!(session.info.attempts.len(), 1);
    assert!(session.info.attempts[0].outcome.is_ok());
    let path: String = session
        .client
        .query_one("SHOW search_path", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(path, "app");
    let timeout: String = session
        .client
        .query_one("SHOW statement_timeout", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(timeout, "30s");
    let idle: String = session
        .client
        .query_one("SHOW idle_in_transaction_session_timeout", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(idle, "65s");
    let role = RoleProfile::load(&session.client).await.unwrap();
    assert_eq!(role.name, scratch.user);
    assert!(session.is_alive().await);
    assert!(session.info.tls_warning().is_some());
    drop(session);
}

#[tokio::test]
async fn a_running_statement_can_be_cancelled_from_another_task() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let settings = scratch.settings(FlagLayer::default());
    let connector = Connector::new(support::shared(settings));
    let session = connector
        .connect()
        .await
        .expect("the scratch database connects");
    let cancel = session.cancel.clone();
    let sleeper =
        tokio::spawn(async move { session.client.batch_execute("SELECT pg_sleep(10)").await });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let tls = ownpg_core::connect::tls::build(
        &scratch.settings(FlagLayer::default()).connection,
        "cancel",
    )
    .unwrap();
    cancel.cancel_query(tls.connector).await.unwrap();
    let outcome = sleeper.await.unwrap();
    let error = outcome.expect_err("the sleep was cancelled");
    assert_eq!(error.code().map(|code| code.code()), Some("57014"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_socket_directory_host_connects_over_the_unix_socket() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let directory = std::env::var("OWNPG_TEST_SOCKET_DIR").unwrap_or_else(|_| "/tmp".to_owned());
    let socket = std::path::Path::new(&directory).join(format!(".s.PGSQL.{}", scratch.port));
    if !socket.exists() {
        eprintln!("{} is absent; the socket test is skipped", socket.display());
        return;
    }
    let settings = scratch.settings(FlagLayer {
        host: Some(directory),
        ..FlagLayer::default()
    });
    let connector = Connector::new(support::shared(settings));
    let session = connector.connect().await.expect("the socket connects");
    assert_eq!(session.info.via, Via::Socket);
    assert!(session.info.tls.is_none());
    assert!(session.info.tls_warning().is_none());
    drop(session);
}

#[tokio::test]
async fn an_unreachable_server_lists_what_was_tried() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let settings = scratch.settings_with(
        FlagLayer::default(),
        &[
            ("OWNPG_HOST", "127.0.0.1"),
            ("OWNPG_PORT", "1"),
            ("OWNPG_CONNECT_TIMEOUT", "2"),
        ],
    );
    let connector = Connector::new(support::shared(settings));
    let error = connector
        .connect()
        .await
        .expect_err("port 1 answers nothing");
    assert_eq!(error.id(), ownpg_core::ErrorId::ConnectFailed);
    assert!(error.remedy().contains("127.0.0.1:1"), "{}", error.remedy());
}
