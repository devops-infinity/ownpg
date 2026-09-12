#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing
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
        .run_read_paged("SELECT id, name FROM big ORDER BY id", true, caps, "tester")
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

    let second = engine.fetch(&cursor, caps, "tester").await.unwrap();
    assert_eq!(second.rows[0][0].as_deref(), Some("101"));
    assert_eq!(second.row_count, 100);
    assert_eq!(second.cursor.as_deref(), Some(cursor.as_str()));

    let third = engine.fetch(&cursor, caps, "tester").await.unwrap();
    assert_eq!(third.row_count, 50);
    assert!(third.cursor.is_none());
    assert!(!third.truncated);
    assert!(engine.open_cursors().await.is_empty());

    let gone = engine.fetch(&cursor, caps, "tester").await.unwrap_err();
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
        .run_read_paged("SELECT count(*) FROM big", true, caps, "tester")
        .await
        .unwrap();
    assert_eq!(result.row_count, 1);
    assert_eq!(result.rows[0][0].as_deref(), Some("5"));
    assert!(result.cursor.is_none());
    assert!(engine.open_cursors().await.is_empty());
    let state = engine
        .run_read("SHOW transaction_read_only", caps)
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
    let error = engine.run_read("DELETE FROM big", caps).await.unwrap_err();
    assert_eq!(error.id(), ErrorId::SqlFailed);
    assert!(error.to_string().contains("25006"), "{error}");
    let after = engine
        .run_read_paged("SELECT count(*) FROM big", true, caps, "tester")
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
        .run_read_paged("SELECT id FROM big ORDER BY id", true, caps, "tester")
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
        .run_read_paged("SELECT 1", true, caps, "tester")
        .await
        .expect("the engine reconnects once");
    assert_eq!(again.rows[0][0].as_deref(), Some("1"));
    let gone = engine.fetch(&cursor, caps, "tester").await.unwrap_err();
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
        &[("OWNPG_HANDLE_EXPIRY", "1"), ("OWNPG_CURSOR_EXPIRY", "1")],
    );
    let engine = Engine::start(Arc::new(settings), Hints::default())
        .await
        .unwrap();
    let caps = Caps {
        row_cap: 100,
        byte_cap: 1_000_000,
    };
    let first = engine
        .run_read_paged("SELECT id FROM big ORDER BY id", true, caps, "tester")
        .await
        .unwrap();
    assert!(first.cursor.is_some());
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    assert_eq!(engine.sweep().await.unwrap(), 1);
    assert!(engine.open_cursors().await.is_empty());
    let state = engine
        .run_read_paged("SELECT 1", true, caps, "tester")
        .await
        .unwrap();
    assert_eq!(state.row_count, 1);
}

#[tokio::test]
async fn a_cursor_answers_only_the_principal_that_opened_it() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let engine = engine_with_rows(&scratch, 250).await;
    let caps = Caps {
        row_cap: 100,
        byte_cap: 1_000_000,
    };
    let first = engine
        .run_read_paged("SELECT id FROM big ORDER BY id", true, caps, "alice")
        .await
        .unwrap();
    let cursor = first.cursor.clone().unwrap();
    let refused = engine.fetch(&cursor, caps, "bob").await.unwrap_err();
    assert_eq!(refused.id(), ErrorId::HandleState);
    assert!(
        refused.to_string().contains("another principal"),
        "{refused}"
    );
    let refused = engine.close_cursor(&cursor, "bob").await.unwrap_err();
    assert!(
        refused.to_string().contains("another principal"),
        "{refused}"
    );
    let second = engine.fetch(&cursor, caps, "alice").await.unwrap();
    assert_eq!(second.row_count, 100);
    engine.close_cursor(&cursor, "alice").await.unwrap();
    assert!(engine.open_cursors().await.is_empty());
}

#[tokio::test]
async fn pooled_mode_pins_every_setting_per_transaction_with_set_local() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app; CREATE TABLE app.big (id int primary key, name text)")
        .await
        .unwrap();
    let settings = scratch.settings_with(
        FlagLayer {
            schema: Some("app".to_owned()),
            ..FlagLayer::default()
        },
        &[("OWNPG_POOLED", "true"), ("OWNPG_STATEMENT_TIMEOUT", "7")],
    );
    let engine = Engine::start(Arc::new(settings), Hints::default())
        .await
        .unwrap();
    assert!(engine.info().await.pooled);
    let caps = Caps {
        row_cap: 10,
        byte_cap: 100_000,
    };
    let path = engine.run_read("SHOW search_path", caps).await.unwrap();
    assert_eq!(path.rows[0][0].as_deref(), Some(r#""""#));
    let timeout = engine
        .run_read("SHOW statement_timeout", caps)
        .await
        .unwrap();
    assert_eq!(timeout.rows[0][0].as_deref(), Some("7s"));
    let encoding = engine.run_read("SHOW client_encoding", caps).await.unwrap();
    assert_eq!(encoding.rows[0][0].as_deref(), Some("UTF8"));
    let inserted = engine
        .run_write(
            "INSERT INTO app.big (id, name) VALUES (1, 'one') RETURNING id",
            caps,
            "tester",
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(inserted.rows[0][0].as_deref(), Some("1"));
    let handle = engine.begin_transaction("tester").await.unwrap();
    let inside = engine
        .run_write(
            "SHOW statement_timeout",
            caps,
            "tester",
            Some(&handle.id),
            false,
        )
        .await
        .unwrap();
    assert_eq!(inside.rows[0][0].as_deref(), Some("7s"));
    engine.commit(&handle.id, "tester").await.unwrap();
    let outside: String = scratch
        .client()
        .await
        .query_one("SHOW statement_timeout", &[])
        .await
        .unwrap()
        .get(0);
    assert_ne!(
        outside, "7s",
        "SET LOCAL must not leak past the transaction"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pooled_handles_give_each_principal_its_own_connection() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app; CREATE TABLE app.big (id int primary key, name text)")
        .await
        .unwrap();
    let settings = scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        ..FlagLayer::default()
    });
    let engine = Engine::start_pooled(Arc::new(settings), Hints::default())
        .await
        .unwrap();
    let caps = Caps {
        row_cap: 10,
        byte_cap: 100_000,
    };
    let alice = engine.begin_transaction("alice").await.unwrap();
    let bob = engine.begin_transaction("bob").await.unwrap();
    assert_ne!(alice.id, bob.id);
    assert_eq!(engine.open_transaction_count().await, 2);
    let again = engine.begin_transaction("alice").await.unwrap_err();
    assert!(again.to_string().contains(&alice.id), "{again}");
    engine
        .run_write(
            "INSERT INTO app.big (id, name) VALUES (1, 'alice')",
            caps,
            "alice",
            Some(&alice.id),
            false,
        )
        .await
        .unwrap();
    let stolen = engine
        .run_write("SELECT 1", caps, "bob", Some(&alice.id), false)
        .await
        .unwrap_err();
    assert!(stolen.to_string().contains("another principal"), "{stolen}");
    let plain = engine
        .run_write(
            "INSERT INTO app.big (id, name) VALUES (2, 'carol') RETURNING id",
            caps,
            "carol",
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(plain.rows[0][0].as_deref(), Some("2"));
    let unseen = engine
        .run_read("SELECT count(*) FROM big", caps)
        .await
        .unwrap();
    assert_eq!(unseen.rows[0][0].as_deref(), Some("1"));
    let saved = engine.savepoint(&alice.id, "alice", "s1").await.unwrap();
    assert_eq!(saved.savepoints, ["s1"]);
    engine.rollback_to(&alice.id, "alice", "s1").await.unwrap();
    let committed = engine.commit(&alice.id, "alice").await.unwrap();
    assert_eq!(committed.statements, 1);
    let rolled = engine.rollback(&bob.id, "bob").await.unwrap();
    assert_eq!(rolled.state.as_str(), "rolled_back");
    assert_eq!(engine.open_transaction_count().await, 0);
    let status = engine.transaction_status(&alice.id).await.unwrap();
    assert_eq!(status.state.as_str(), "committed");
    let seen = engine
        .run_read("SELECT count(*) FROM big", caps)
        .await
        .unwrap();
    assert_eq!(seen.rows[0][0].as_deref(), Some("2"));
    engine.release_everything().await.unwrap();
}

#[tokio::test]
async fn the_second_local_connection_closes_with_the_handle() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let engine = engine_with_rows(&scratch, 5).await;
    let caps = Caps {
        row_cap: 10,
        byte_cap: 100_000,
    };
    let watcher = scratch.client().await;
    let count = || async {
        let row = watcher
            .query_one(
                "SELECT count(*)::int4 FROM pg_stat_activity WHERE datname = current_database() AND application_name LIKE 'ownpg/%'",
                &[],
            )
            .await
            .unwrap();
        row.get::<_, i32>(0)
    };
    assert_eq!(count().await, 1);
    let handle = engine.begin_transaction("tester").await.unwrap();
    let read = engine
        .run_read("SELECT count(*) FROM big", caps)
        .await
        .unwrap();
    assert_eq!(read.rows[0][0].as_deref(), Some("5"));
    assert_eq!(count().await, 2, "the read opened the second connection");
    engine.commit(&handle.id, "tester").await.unwrap();
    for _ in 0..50 {
        if count().await == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        count().await,
        1,
        "the second connection closes with the handle"
    );
}
