#![allow(
    clippy::expect_used,
    reason = "clippy.toml exempts test modules, and an integration test is a separate crate it cannot reach"
)]

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
    for command in ["man", "completions"] {
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
fn no_command_at_all_is_a_usage_error() {
    ownpg()
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Usage:"));
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
            "try: Use one of: man, completions.",
        ))
        .stderr(predicate::str::contains("code: command.unknown"));
}
