#![allow(clippy::expect_used)]

use assert_cmd::Command;
use predicates::prelude::*;

fn ownpg() -> Command {
    let mut command = Command::cargo_bin("ownpg").expect("the binary is built for the test run");
    command.env("NO_COLOR", "1");
    for variable in [
        "RUST_LOG",
        "RUST_BACKTRACE",
        "CI",
        "GITHUB_ACTIONS",
        "COLUMNS",
        "LINES",
        "FORCE_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
    ] {
        command.env_remove(variable);
    }
    for (name, _) in std::env::vars() {
        if name.starts_with("OWNPG_") || name.starts_with("PG") {
            command.env_remove(&name);
        }
    }
    command
}

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary home"),
        }
    }

    fn apply(&self, command: &mut Command) {
        command.env("HOME", self.dir.path());
        command.env("XDG_CONFIG_HOME", self.dir.path().join("config"));
        command.env("XDG_DATA_HOME", self.dir.path().join("data"));
        command.env("XDG_CACHE_HOME", self.dir.path().join("cache"));
    }

    fn apply_std(&self, command: &mut std::process::Command) {
        command.env("HOME", self.dir.path());
        command.env("XDG_CONFIG_HOME", self.dir.path().join("config"));
        command.env("XDG_DATA_HOME", self.dir.path().join("data"));
        command.env("XDG_CACHE_HOME", self.dir.path().join("cache"));
    }
}

struct Live {
    host: String,
    port: String,
    user: String,
    database: String,
}

fn live() -> Option<Live> {
    let dsn = std::env::var("OWNPG_TEST_DSN").ok()?;
    let trimmed = dsn.strip_prefix("postgresql://")?;
    let (credentials, rest) = trimmed.split_once('@')?;
    let (address, database) = rest.split_once('/')?;
    let (host, port) = address.split_once(':').unwrap_or((address, "5432"));
    let user = credentials.split(':').next()?.to_owned();
    Some(Live {
        host: host.to_owned(),
        port: port.to_owned(),
        user,
        database: database.to_owned(),
    })
}

fn connected(live: &Live, home: &Home) -> Command {
    let mut command = ownpg();
    home.apply(&mut command);
    command.env("OWNPG_HOST", &live.host);
    command.env("OWNPG_PORT", &live.port);
    command.env("OWNPG_USER", &live.user);
    command.env("OWNPG_DATABASE", &live.database);
    command
}

#[test]
fn help_goes_to_stdout_and_exits_zero() {
    ownpg()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Model Context Protocol"))
        .stdout(predicate::str::contains("EXIT CODES:"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn version_goes_to_stdout_and_exits_zero() {
    ownpg()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "ownpg {} (commit ",
            env!("CARGO_PKG_VERSION")
        )))
        .stdout(predicate::str::contains(", built "))
        .stderr(predicate::str::is_empty());
}

#[test]
fn every_subcommand_answers_its_own_help() {
    for command in ["serve", "doctor", "config", "man", "completions"] {
        ownpg()
            .args([command, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains(command));
    }
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    ownpg()
        .arg("--not-a-real-flag")
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("--not-a-real-flag"));
}

#[test]
fn no_command_means_serve_and_a_missing_database_is_a_usage_error() {
    let home = Home::new();
    let mut command = ownpg();
    home.apply(&mut command);
    command
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no database was named"))
        .stderr(predicate::str::contains("--database"));
}

#[test]
fn a_public_http_bind_without_authentication_is_refused_before_connecting() {
    let home = Home::new();
    let mut command = ownpg();
    home.apply(&mut command);
    command
        .args([
            "serve",
            "--http",
            "--bind",
            "0.0.0.0:0",
            "--auth",
            "none",
            "-d",
            "app",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--auth"))
        .stderr(predicate::str::contains("code: config.invalid"));
    let mut bearer = ownpg();
    home.apply(&mut bearer);
    bearer
        .args([
            "serve",
            "--http",
            "--bind",
            "0.0.0.0:0",
            "--auth",
            "bearer",
            "-d",
            "app",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("OWNPG_BEARER_TOKENS"))
        .stderr(predicate::str::contains("code: config.invalid"));
}

#[test]
fn config_path_and_init_write_under_the_config_directory() {
    let home = Home::new();
    let mut path = ownpg();
    home.apply(&mut path);
    path.args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("profiles.toml"))
        .stdout(predicate::str::contains("data:"))
        .stdout(predicate::str::contains("cache:"));

    let mut dry = ownpg();
    home.apply(&mut dry);
    dry.args(["config", "init", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("format = 1"))
        .stdout(predicate::str::contains("[profiles.local]"));

    let mut init = ownpg();
    home.apply(&mut init);
    init.args(["config", "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("profiles.toml"));

    let mut again = ownpg();
    home.apply(&mut again);
    again
        .args(["config", "init"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--force"));

    let mut show = ownpg();
    home.apply(&mut show);
    show.args(["config", "show", "--profile", "local", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"format_version\": 1"))
        .stdout(predicate::str::contains("\"name\": \"database\""))
        .stdout(predicate::str::contains("your-database"));

    let mut unknown = ownpg();
    home.apply(&mut unknown);
    unknown
        .args(["config", "show", "--profile", "nope"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("code: profile.unknown"));
}

#[test]
fn config_show_masks_a_password_from_the_environment() {
    let home = Home::new();
    let mut show = ownpg();
    home.apply(&mut show);
    show.env("PGPASSWORD", "hunter2")
        .args(["config", "show", "-d", "app"])
        .assert()
        .success()
        .stdout(predicate::str::contains("password = set (libpq)"))
        .stdout(predicate::str::contains("hunter2").not());
}

#[test]
fn doctor_reports_the_live_connection_in_json_and_text() {
    let Some(live) = live() else {
        return;
    };
    let home = Home::new();
    let mut json = connected(&live, &home);
    let output = json
        .args(["doctor", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let verdict: serde_json::Value = serde_json::from_slice(&output).expect("json report");
    assert_eq!(verdict["format_version"], 1);
    assert!(
        verdict["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "public_schema"),
        "{verdict}"
    );
    assert!(
        verdict["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "connection" && check["status"] == "ok"),
        "{verdict}"
    );
    assert_eq!(verdict["report"]["database"], live.database);
    assert_eq!(verdict["report"]["tools"].as_array().unwrap().len(), 7);

    let mut text = connected(&live, &home);
    text.arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("check connection [ok]"))
        .stdout(predicate::str::contains("verdict:"));

    let mut unreachable = connected(&live, &home);
    unreachable
        .args(["doctor", "--port", "1", "--format", "json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"status\": \"failed\""));
}

#[test]
fn serve_answers_a_client_over_stdio_and_stops_on_eof() {
    let Some(live) = live() else {
        return;
    };
    let home = Home::new();
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("ownpg"));
    command.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        command.env("PATH", path);
    }
    home.apply_std(&mut command);
    command.env("OWNPG_HOST", &live.host);
    command.env("OWNPG_PORT", &live.port);
    command.env("OWNPG_USER", &live.user);
    command.env("OWNPG_DATABASE", &live.database);
    command.args(["serve", "--mode", "read-only"]);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("the server starts");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead;
        let mut lines = Vec::new();
        for line in std::io::BufReader::new(stdout).lines() {
            match line {
                Ok(line) => lines.push(line),
                Err(_) => break,
            }
        }
        lines
    });
    use std::io::Write;
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "cli-test", "version": "0"}}})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "pg_run_query", "arguments": {"sql": "SELECT 1 AS one"}}})
    )
    .unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let started = std::time::Instant::now();
    let status = child.wait().expect("the server exits");
    assert!(status.success(), "{status:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let lines = reader.join().unwrap();
    let responses: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("json-rpc lines only"))
        .collect();
    let by_id = |id: u64| responses.iter().find(|r| r["id"] == id).cloned().unwrap();
    assert_eq!(by_id(1)["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(by_id(2)["result"]["tools"].as_array().unwrap().len(), 7);
    assert_eq!(by_id(3)["result"]["structuredContent"]["rows"][0][0], "1");
    let data = std::fs::read_dir(home.dir.path().join("data").join("ownpg")).unwrap();
    let audit = data
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .expect("an audit file");
    let content = std::fs::read_to_string(audit).unwrap();
    assert!(content.contains("\"tool\":\"pg_run_query\""));
}

#[test]
fn a_missing_shell_name_is_a_usage_error() {
    ownpg()
        .arg("completions")
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("<SHELL>"));
}

#[test]
fn the_completion_script_is_written_for_every_shell_offered() {
    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        ownpg()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("ownpg"))
            .stderr(predicate::str::is_empty());
    }
}

#[test]
fn the_manual_is_written_for_the_tool_and_for_each_command() {
    ownpg()
        .arg("man")
        .assert()
        .success()
        .stdout(predicate::str::contains(".TH ownpg"))
        .stderr(predicate::str::is_empty());
    for command in ["man", "completions"] {
        ownpg()
            .args(["man", command])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(".TH ownpg-{command}")));
    }
}

#[test]
fn an_unknown_manual_command_carries_its_error_id_and_the_usage_class() {
    ownpg()
        .args(["man", "nope"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("error: `nope` is not a command"))
        .stderr(predicate::str::contains(
            "try: Use one of: serve, doctor, config, man, completions.",
        ))
        .stderr(predicate::str::contains("code: command.unknown"));
}

#[test]
fn the_first_discover_answer_arrives_quickly() {
    let Some(live) = live() else {
        return;
    };
    let home = Home::new();
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("ownpg"));
    command.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        command.env("PATH", path);
    }
    home.apply_std(&mut command);
    command.env("OWNPG_HOST", &live.host);
    command.env("OWNPG_PORT", &live.port);
    command.env("OWNPG_USER", &live.user);
    command.env("OWNPG_DATABASE", &live.database);
    command.arg("serve");
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::null());
    let started = std::time::Instant::now();
    let mut child = command.spawn().expect("the server starts");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    use std::io::{BufRead, Write};
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        }}})
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let elapsed = started.elapsed();
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());
    let response: serde_json::Value = serde_json::from_str(&line).expect("a discover answer");
    assert_eq!(response["id"], 1, "{line}");
    assert!(response["result"]["supportedVersions"].is_array(), "{line}");
    eprintln!(
        "startup to the first discover answer: {} ms",
        elapsed.as_millis()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "startup took {elapsed:?}"
    );
}

#[test]
fn the_platform_keychain_holds_the_password_and_the_ssh_passphrase() {
    if std::env::var("OWNPG_TEST_KEYCHAIN").is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let profiles = dir.path().join("profiles.toml");
    let profile = format!("kc-test-{}", std::process::id());
    std::fs::write(
        &profiles,
        format!(
            "format = 1\n\n[profiles.{profile}]\ndatabase = \"app\"\n\n[profiles.{profile}.ssh]\nhost = \"bastion.test\"\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&profiles, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let isolated = || {
        let mut command = ownpg();
        command.env("OWNPG_CONFIG", &profiles);
        command
    };

    isolated()
        .args(["config", "set-password", &profile])
        .write_stdin("keychain-secret-1\n")
        .assert()
        .success();
    isolated()
        .args(["config", "show", "--profile", &profile])
        .assert()
        .success()
        .stdout(predicate::str::contains("password = set (profile)"))
        .stdout(predicate::str::contains("keychain-secret-1").not());
    let stored = std::fs::read_to_string(&profiles).unwrap();
    assert!(stored.contains("password_keychain = true"), "{stored}");
    assert!(!stored.contains("keychain-secret-1"), "{stored}");

    isolated()
        .args(["config", "set-ssh-passphrase", &profile])
        .write_stdin("phrase-secret-2\n")
        .assert()
        .success();
    isolated()
        .args(["config", "show", "--profile", &profile])
        .assert()
        .success()
        .stdout(predicate::str::contains("ssh.password = set (profile)"))
        .stdout(predicate::str::contains("phrase-secret-2").not());
    let stored = std::fs::read_to_string(&profiles).unwrap();
    assert!(stored.contains("passphrase_keychain = true"), "{stored}");

    isolated()
        .args(["config", "unset-password", &profile])
        .assert()
        .success();
    isolated()
        .args(["config", "unset-ssh-passphrase", &profile])
        .assert()
        .success();
    isolated()
        .args(["config", "show", "--profile", &profile])
        .assert()
        .success()
        .stdout(predicate::str::contains("password = set").not());
}
