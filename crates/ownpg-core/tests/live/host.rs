use crate::support;

use std::path::PathBuf;
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
    output_dir: PathBuf,
    database: String,
}

fn test_pg_bindir() -> Option<PathBuf> {
    std::env::var_os("OWNPG_TEST_PG_BINDIR").map(PathBuf::from)
}

async fn rig(scratch: &support::Scratch, output_dir: Option<PathBuf>) -> Rig {
    let settings = Arc::new(scratch.settings(FlagLayer {
        schema: Some("app".to_owned()),
        mode: Some(Mode::ReadWrite),
        tools: Some(vec![ToolGroup::Host]),
        pg_bindir: test_pg_bindir(),
        output_dir: output_dir.clone(),
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
            Principal::local(Some("host")),
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
        output_dir: output_dir.unwrap_or_default(),
        database: scratch.database.clone(),
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

    async fn failed(&self, tool: &str, arguments: Value) -> Value {
        let result = self.call(tool, arguments).await;
        assert_eq!(result.is_error, Some(true), "{tool}: {result:?}");
        result.structured_content.unwrap_or(Value::Null)
    }

    async fn names(&self) -> Vec<String> {
        self.client
            .list_tools(None)
            .await
            .unwrap()
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    async fn finish(self) {
        drop(self.client);
        self.server_task
            .await
            .unwrap()
            .expect("the server stops cleanly");
    }
}

fn program_major(path: &str) -> Option<u32> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .last()?
        .split('.')
        .next()?
        .parse()
        .ok()
}

#[cfg(unix)]
fn permission_bits(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_tools_dump_restore_and_report_their_programs() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    let client = scratch.client().await;
    client
        .batch_execute(
            "CREATE SCHEMA app; CREATE SCHEMA other; \
             CREATE TABLE app.items (id int primary key, label text); \
             INSERT INTO app.items VALUES (1, 'one'), (2, 'two'), (3, 'three'); \
             CREATE TABLE other.secret (id int);",
        )
        .await
        .unwrap();
    let output_dir = tempfile::tempdir().unwrap();
    let rig = rig(&scratch, Some(output_dir.path().to_path_buf())).await;
    let names = rig.names().await;
    for expected in [
        "pg_dump",
        "pg_dumpall_globals",
        "pg_restore",
        "pg_basebackup",
        "pg_upgrade_check",
    ] {
        assert!(names.contains(&expected.to_owned()), "{expected} missing");
    }

    let doctor = rig.ok("pg_doctor", json!({})).await;
    let programs = doctor["host_programs"].as_array().unwrap();
    assert_eq!(programs.len(), 5);
    let dump_program = programs
        .iter()
        .find(|program| program["name"] == "pg_dump")
        .unwrap();
    let dump_path = dump_program["path"].as_str().unwrap().to_owned();
    assert!(PathBuf::from(&dump_path).is_absolute());
    assert!(
        dump_program["version"]
            .as_str()
            .unwrap()
            .starts_with("pg_dump (PostgreSQL)")
    );
    let server_major: u32 = doctor["server_version"]
        .as_str()
        .unwrap()
        .split('.')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let client_major = program_major(&dump_path).unwrap();

    let dry = rig
        .ok(
            "pg_dump",
            json!({"file": "app.dump", "format": "custom", "tables": ["items"], "dry_run": true}),
        )
        .await;
    assert_eq!(dry["dry_run"], true);
    let arguments: Vec<&str> = dry["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert!(arguments.contains(&format!("--dbname={}", rig.database).as_str()));
    assert!(arguments.contains(&"--schema=app"));
    assert!(arguments.contains(&"--format=custom"));
    assert!(arguments.contains(&"--table=\"app\".\"items\""));
    assert!(arguments.contains(&"--no-password"));
    let file_argument = format!("--file={}", rig.output_dir.join("app.dump").display());
    assert!(arguments.contains(&file_argument.as_str()), "{arguments:?}");
    assert!(!rig.output_dir.join("app.dump").exists());

    for bad in [
        "../escape",
        ".hidden",
        "a b",
        "",
        "con",
        "NUL.dump",
        "trailing.",
        "x.ownpg-partial",
    ] {
        let failure = rig.failed("pg_dump", json!({"file": bad})).await;
        assert_eq!(failure["code"], "argument.invalid", "{bad:?}: {failure}");
    }
    let failure = rig
        .failed("pg_dump", json!({"file": "x", "tables": ["other.secret"]}))
        .await;
    assert_eq!(failure["code"], "statement.refused", "{failure}");
    let failure = rig
        .failed("pg_dump", json!({"file": "x", "jobs": "4"}))
        .await;
    assert_eq!(failure["code"], "argument.invalid", "{failure}");
    let failure = rig
        .failed("pg_restore", json!({"file": "missing.dump"}))
        .await;
    assert_eq!(failure["code"], "argument.invalid", "{failure}");
    let failure = rig
        .failed(
            "pg_upgrade_check",
            json!({"old_bindir": "/nonexistent/bin", "old_datadir": "/nonexistent/old", "new_datadir": "/nonexistent/new"}),
        )
        .await;
    assert_eq!(failure["code"], "argument.invalid", "{failure}");

    let old_bindir = PathBuf::from(&dump_path).parent().unwrap().to_path_buf();
    let upgrade = rig
        .ok(
            "pg_upgrade_check",
            json!({
                "old_bindir": old_bindir.display().to_string(),
                "old_datadir": rig.output_dir.display().to_string(),
                "new_datadir": rig.output_dir.display().to_string(),
                "old_port": "5433",
                "method": "link",
                "dry_run": true
            }),
        )
        .await;
    let arguments: Vec<&str> = upgrade["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(arguments[0], "--check");
    assert!(arguments.contains(&"--old-port=5433"));
    assert!(arguments.contains(&"--link"));

    if client_major < server_major {
        eprintln!(
            "pg_dump {client_major} is older than the server ({server_major}); the real runs are skipped"
        );
        rig.finish().await;
        return;
    }

    let dumped = rig
        .ok(
            "pg_dump",
            json!({"file": "app.dump", "format": "custom", "tables": ["items"]}),
        )
        .await;
    assert_eq!(dumped["exit_code"], 0, "{dumped}");
    let dump_file = rig.output_dir.join("app.dump");
    assert!(dumped["output_bytes"].as_u64().unwrap() > 0);
    assert_eq!(dumped["output"], dump_file.display().to_string());
    assert!(!rig.output_dir.join("app.dump.ownpg-partial").exists());
    #[cfg(unix)]
    assert_eq!(permission_bits(&dump_file), 0o600);
    let repeat = rig
        .failed("pg_dump", json!({"file": "app.dump", "format": "custom"}))
        .await;
    assert_eq!(repeat["code"], "argument.invalid", "{repeat}");
    assert!(dump_file.metadata().unwrap().len() > 0);

    let restore_dry = rig
        .ok(
            "pg_restore",
            json!({"file": "app.dump", "tables": ["items"], "dry_run": true}),
        )
        .await;
    let restore_arguments: Vec<&str> = restore_dry["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert!(
        restore_arguments.contains(&"--table=items"),
        "{restore_arguments:?}"
    );
    assert!(
        !restore_arguments
            .iter()
            .any(|argument| argument.contains('"')),
        "{restore_arguments:?}"
    );

    let plain = rig
        .ok(
            "pg_dump",
            json!({"file": "app.sql", "schema_only": true, "no_owner": true}),
        )
        .await;
    assert_eq!(plain["exit_code"], 0, "{plain}");
    let text = std::fs::read_to_string(rig.output_dir.join("app.sql")).unwrap();
    assert!(text.contains("CREATE TABLE app.items"), "{text}");
    assert!(!text.contains("secret"), "{text}");

    let globals = rig
        .ok(
            "pg_dumpall_globals",
            json!({"file": "globals.sql", "no_role_passwords": true}),
        )
        .await;
    assert_eq!(globals["exit_code"], 0, "{globals}");
    let text = std::fs::read_to_string(rig.output_dir.join("globals.sql")).unwrap();
    assert!(text.contains("CREATE ROLE"), "{text}");
    #[cfg(unix)]
    assert_eq!(permission_bits(&rig.output_dir.join("globals.sql")), 0o600);

    client.batch_execute("DROP TABLE app.items").await.unwrap();
    let restored = rig.ok("pg_restore", json!({"file": "app.dump"})).await;
    assert_eq!(restored["exit_code"], 0, "{restored}");
    let count = rig
        .ok("pg_run_query", json!({"sql": "SELECT count(*) FROM items"}))
        .await;
    assert_eq!(count["rows"][0][0], "3");

    let needs_confirm = rig
        .failed("pg_restore", json!({"file": "app.dump", "clean": true}))
        .await;
    assert_eq!(needs_confirm["code"], "confirmation.required");
    let cleaned = rig
        .ok(
            "pg_restore",
            json!({"file": "app.dump", "clean": true, "if_exists": true, "confirm": true}),
        )
        .await;
    assert_eq!(cleaned["exit_code"], 0, "{cleaned}");

    let backup = rig
        .failed(
            "pg_basebackup",
            json!({"directory": "base", "wal_method": "none"}),
        )
        .await;
    assert_eq!(backup["code"], "subprocess.failed", "{backup}");
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_tools_refuse_to_run_without_an_output_directory() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    scratch
        .client()
        .await
        .batch_execute("CREATE SCHEMA app")
        .await
        .unwrap();
    let rig = rig(&scratch, None).await;
    let failure = rig.failed("pg_dump", json!({"file": "app.dump"})).await;
    assert_eq!(failure["code"], "config.invalid", "{failure}");
    assert!(failure["message"].as_str().unwrap().contains("output_dir"));
    rig.finish().await;
}
