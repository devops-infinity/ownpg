#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    reason = "clippy.toml exempts test modules, and an integration test is a separate crate it cannot reach"
)]

mod support;

use std::sync::Arc;

use ownpg_core::ErrorId;
use ownpg_core::config::FlagLayer;
use ownpg_core::connect::ssh::Hints;
use ownpg_core::engine::Engine;
use ownpg_core::shape::{Caps, Truncation};

async fn engine_with_rows(scratch: &support::Scratch, rows: i32) -> Engine {
    let client = scratch.client().await;
    client
        .batch_execute(&format!(
            "CREATE SCHEMA app; CREATE TABLE app.big (id int primary key, name text); INSERT INTO app.big SELECT g, 'row ' || g FROM generate_series(1, {rows}) g; ANALYZE app.big;"
        ))
        .await
        .unwrap();
    let settings = scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        ..FlagLayer::default()
    });
    Engine::start(Arc::new(settings), Hints::default())
        .await
        .expect("the engine starts")
}

#[tokio::test]
async fn a_large_select_is_paged_through_a_cursor_in_order() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let engine = engine_with_rows(&scratch, 250).await;
    let caps = Caps {
        row_cap: 100,
        byte_cap: 1_000_000,
    };
    let first = engine
        .run_read("SELECT id, name FROM big ORDER BY id", true, caps)
        .await
        .unwrap();
    assert_eq!(first.row_count, 100);
    assert!(first.truncated);
    assert_eq!(first.truncated_by, Some(Truncation::RowCap));
    assert_eq!(first.columns[0].type_name, "int4");
    assert_eq!(first.columns[1].type_name, "text");
    assert_eq!(first.rows[0][0].as_deref(), Some("1"));
    assert_eq!(first.rows[99][0].as_deref(), Some("100"));
    assert!(first.estimate.is_some());
    let cursor = first.cursor.clone().expect("a cursor handle");
    assert_eq!(engine.open_cursors().await.len(), 1);

    let second = engine.fetch(&cursor, caps).await.unwrap();
    assert_eq!(second.rows[0][0].as_deref(), Some("101"));
    assert_eq!(second.row_count, 100);
    assert_eq!(second.cursor.as_deref(), Some(cursor.as_str()));

    let third = engine.fetch(&cursor, caps).await.unwrap();
    assert_eq!(third.row_count, 50);
    assert!(third.cursor.is_none());
    assert!(!third.truncated);
    assert!(engine.open_cursors().await.is_empty());

    let gone = engine.fetch(&cursor, caps).await.unwrap_err();
    assert_eq!(gone.id(), ErrorId::HandleState);
}

#[tokio::test]
async fn a_small_select_leaves_no_cursor_and_the_transaction_ends() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let engine = engine_with_rows(&scratch, 5).await;
    let caps = Caps {
        row_cap: 100,
        byte_cap: 1_000_000,
    };
    let result = engine
        .run_read("SELECT count(*) FROM big", true, caps)
        .await
        .unwrap();
    assert_eq!(result.row_count, 1);
    assert_eq!(result.rows[0][0].as_deref(), Some("5"));
    assert!(result.cursor.is_none());
    assert!(engine.open_cursors().await.is_empty());
    let state = engine
        .run_read("SHOW transaction_read_only", false, caps)
        .await
        .unwrap();
    assert_eq!(state.rows[0][0].as_deref(), Some("on"));
}

#[tokio::test]
async fn a_write_that_slips_past_the_classifier_is_stopped_by_the_read_only_transaction() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let engine = engine_with_rows(&scratch, 3).await;
    let caps = Caps {
        row_cap: 10,
        byte_cap: 10_000,
    };
    let error = engine
        .run_read("DELETE FROM big", false, caps)
        .await
        .unwrap_err();
    assert_eq!(error.id(), ErrorId::SqlFailed);
    assert!(error.to_string().contains("25006"), "{error}");
    let after = engine
        .run_read("SELECT count(*) FROM big", true, caps)
        .await
        .unwrap();
    assert_eq!(after.rows[0][0].as_deref(), Some("3"));
}

#[tokio::test]
async fn a_lost_connection_is_reconnected_once_and_cursors_are_gone() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let engine = engine_with_rows(&scratch, 300).await;
    let caps = Caps {
        row_cap: 100,
        byte_cap: 1_000_000,
    };
    let first = engine
        .run_read("SELECT id FROM big ORDER BY id", true, caps)
        .await
        .unwrap();
    let cursor = first.cursor.clone().unwrap();
    let info = engine.info().await;
    let killer = scratch.client().await;
    killer
        .execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1 AND datname = $2 AND pid <> pg_backend_pid()",
            &[&info_application_name(&engine), &scratch.database],
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let again = engine
        .run_read("SELECT 1", true, caps)
        .await
        .expect("the engine reconnects once");
    assert_eq!(again.rows[0][0].as_deref(), Some("1"));
    let gone = engine.fetch(&cursor, caps).await.unwrap_err();
    assert_eq!(gone.id(), ErrorId::HandleState);
    assert_eq!(info.database, scratch.database);
}

fn info_application_name(engine: &Engine) -> String {
    engine.settings().connection.application_name.value.clone()
}

#[tokio::test]
async fn expired_cursors_are_swept_and_the_transaction_ends() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let client = scratch.client().await;
    client
        .batch_execute("CREATE SCHEMA app; CREATE TABLE app.big AS SELECT g AS id FROM generate_series(1, 300) g;")
        .await
        .unwrap();
    let settings = scratch.settings_with(
        FlagLayer {
            schema: Some("app".to_owned()),
            ..FlagLayer::default()
        },
        &[("OWNPG_HANDLE_EXPIRY", "1")],
    );
    let engine = Engine::start(Arc::new(settings), Hints::default())
        .await
        .unwrap();
    let caps = Caps {
        row_cap: 100,
        byte_cap: 1_000_000,
    };
    let first = engine
        .run_read("SELECT id FROM big ORDER BY id", true, caps)
        .await
        .unwrap();
    assert!(first.cursor.is_some());
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    assert_eq!(engine.sweep().await.unwrap(), 1);
    assert!(engine.open_cursors().await.is_empty());
    let state = engine.run_read("SELECT 1", true, caps).await.unwrap();
    assert_eq!(state.row_count, 1);
}
