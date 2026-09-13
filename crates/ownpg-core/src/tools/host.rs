use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::confirm::Verdict;
use super::ddl::{number, scoped_name};
use super::{AuditFacts, Call, Outcome, Route, ToolFailure, ToolOutput, route};
use crate::audit::{Decision, sha256_hex};
use crate::classify::{Classification, StatementClass};
use crate::config::Settings;
use crate::connect::{Endpoint, Via};
use crate::error::{Error, Result};
use crate::shape::UNTRUSTED_NOTICE;
use crate::tool_specs;

pub const PROGRAMS: [&str; 5] = [
    "pg_dump",
    "pg_dumpall",
    "pg_restore",
    "pg_basebackup",
    "pg_upgrade",
];
pub const TAIL_LINES: usize = 50;
pub const PROGRESS_LINES: usize = 1_000;
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

const DUMP_DESCRIPTION: &str = "Run pg_dump on the host against the connected database, pinned to the scoped schema, and write the archive into the configured output_dir with mode 0600. Choose the plain, custom, directory, or tar format, restrict it to named tables, and dump only the data or only the definitions. Progress comes from the program's own verbose output when the client sends a progress token. The password never reaches the command line.";
const DUMPALL_DESCRIPTION: &str = "Run pg_dumpall --globals-only on the host to dump roles and tablespaces (no database contents) into the configured output_dir with mode 0600. The maintenance connection uses the connected database.";
const RESTORE_DESCRIPTION: &str = "Run pg_restore on the host to load a custom, directory, or tar archive from the configured output_dir into the connected database, pinned to the scoped schema. clean drops the objects first and needs confirm or a confirmation prompt. Runs only in a write mode.";
const BASEBACKUP_DESCRIPTION: &str = "Run pg_basebackup on the host to take a physical copy of the whole cluster into a new directory under the configured output_dir (mode 0700). The connected role needs the REPLICATION attribute. Progress comes from the program's own progress output when the client sends a progress token.";
const UPGRADE_CHECK_DESCRIPTION: &str = "Run pg_upgrade --check on the host to test whether the old cluster can be upgraded to the new binaries without changing anything. Both data directories and the old binary directory must be absolute paths on this host; the new binary directory defaults to the one that holds pg_upgrade. Logs land in the configured output_dir.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HostProgram {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[must_use]
pub fn find_program(settings: &Settings, name: &str) -> Option<PathBuf> {
    let file_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    if let Some(bindir) = &settings.pg_bindir {
        let candidate = bindir.value.join(&file_name);
        return is_executable(&candidate).then_some(candidate);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|entry| entry.is_absolute())
        .map(|entry| entry.join(&file_name))
        .find(|candidate| is_executable(candidate))
}

const FORWARDED_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "PGCLIENTENCODING",
    "PGTZ",
    "PGDATESTYLE",
    "PGGEQO",
    "PGSYSCONFDIR",
    "PGLOCALEDIR",
    "PGSSLNEGOTIATION",
    "PGSSLSNI",
    "PGSSLCERTMODE",
    "PGSSLCRL",
    "PGSSLCRLDIR",
    "PGGSSENCMODE",
    "PGREQUIREPEER",
    "PGTARGETSESSIONATTRS",
];

fn base_command(program: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.env_clear();
    for key in FORWARDED_ENVIRONMENT {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.stdin(Stdio::null());
    command.kill_on_drop(true);
    command
}

pub async fn program_report(settings: &Settings) -> Vec<HostProgram> {
    let mut report = Vec::new();
    for name in PROGRAMS {
        let Some(path) = find_program(settings, name) else {
            report.push(HostProgram {
                name: name.to_owned(),
                path: None,
                version: None,
            });
            continue;
        };
        let mut command = base_command(&path);
        command.arg("--version");
        command.stdout(Stdio::piped());
        command.stderr(Stdio::null());
        let version = match tokio::time::timeout(VERSION_TIMEOUT, command.output()).await {
            Ok(Ok(output)) if output.status.success() => String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned),
            _ => None,
        };
        report.push(HostProgram {
            name: name.to_owned(),
            path: Some(path.display().to_string()),
            version,
        });
    }
    report
}

struct ConnectionEnv {
    vars: Vec<(String, String)>,
    _passfile: Option<tempfile::NamedTempFile>,
}

fn pgpass_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(':', "\\:")
}

async fn connection_env(call: &Call) -> Result<ConnectionEnv> {
    let settings = call.settings();
    let info = call.engine().info().await;
    let endpoint = match (info.via, info.endpoint) {
        (Via::Ssh, _) | (_, None) => {
            return Err(Error::ConfigInvalid {
                setting: "ssh".to_owned(),
                value: info.target,
                detail: "host programs cannot use the in-process SSH tunnel; connect directly, or run the program on the far host".to_owned(),
            });
        }
        (_, Some(endpoint)) => endpoint,
    };
    let connection = &settings.connection;
    let mut vars = Vec::new();
    match endpoint {
        Endpoint::Socket { directory, port } => {
            vars.push(("PGHOST".to_owned(), directory.display().to_string()));
            vars.push(("PGPORT".to_owned(), port.to_string()));
        }
        Endpoint::Tcp { host, port } => {
            vars.push(("PGHOST".to_owned(), host));
            vars.push(("PGPORT".to_owned(), port.to_string()));
            vars.push((
                "PGSSLMODE".to_owned(),
                connection.sslmode.value.as_str().to_owned(),
            ));
            vars.push((
                "PGCHANNELBINDING".to_owned(),
                connection.channel_binding.value.as_str().to_owned(),
            ));
            for (key, path) in [
                ("PGSSLROOTCERT", &connection.sslrootcert),
                ("PGSSLCERT", &connection.sslcert),
                ("PGSSLKEY", &connection.sslkey),
            ] {
                if let Some(path) = path {
                    vars.push((key.to_owned(), path.value.display().to_string()));
                }
            }
        }
    }
    vars.push(("PGUSER".to_owned(), info.user.clone()));
    vars.push(("PGDATABASE".to_owned(), settings.database.value.clone()));
    vars.push((
        "PGCONNECT_TIMEOUT".to_owned(),
        connection
            .connect_timeout
            .value
            .as_secs()
            .max(1)
            .to_string(),
    ));
    vars.push((
        "PGAPPNAME".to_owned(),
        connection.application_name.value.clone(),
    ));
    if let Some(options) = &connection.options {
        vars.push(("PGOPTIONS".to_owned(), options.value.clone()));
    }
    let mut passfile = None;
    if let Some(password) = &connection.password {
        crate::config::profile::create_private_directory(&settings.paths.data_dir).map_err(
            |error| Error::ConfigUnwritable {
                path: settings.paths.data_dir.clone(),
                source: error,
            },
        )?;
        sweep_stale_passfiles(&settings.paths.data_dir);
        let mut file = tempfile::Builder::new()
            .prefix(PASSFILE_PREFIX)
            .tempfile_in(&settings.paths.data_dir)
            .map_err(|error| Error::ConfigUnwritable {
                path: settings.paths.data_dir.clone(),
                source: error,
            })?;
        restrict_file(file.path())?;
        std::io::Write::write_all(
            &mut file,
            format!(
                "*:*:*:{}:{}\n",
                pgpass_escape(&info.user),
                pgpass_escape(password.value.expose())
            )
            .as_bytes(),
        )
        .map_err(|error| Error::ConfigUnwritable {
            path: file.path().to_path_buf(),
            source: error,
        })?;
        vars.push(("PGPASSFILE".to_owned(), file.path().display().to_string()));
        passfile = Some(file);
    } else {
        vars.push(("PGPASSFILE".to_owned(), String::new()));
    }
    Ok(ConnectionEnv {
        vars,
        _passfile: passfile,
    })
}

const PASSFILE_PREFIX: &str = "ownpg-pgpass-";
const STALE_PASSFILE_AGE: Duration = Duration::from_secs(86_400);

pub fn sweep_stale_passfiles(data_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(PASSFILE_PREFIX) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_PASSFILE_AGE);
        if stale && let Err(error) = std::fs::remove_file(entry.path()) {
            tracing::debug!(%error, "a stale password file could not be removed");
        }
    }
}

fn restrict_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| Error::ConfigUnwritable {
                path: path.to_path_buf(),
                source: error,
            },
        )?;
    }
    let _ = path;
    Ok(())
}

fn restrict_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| Error::ConfigUnwritable {
                path: path.to_path_buf(),
                source: error,
            },
        )?;
    }
    let _ = path;
    Ok(())
}

fn output_dir(call: &Call) -> Result<PathBuf> {
    let Some(dir) = &call.settings().output_dir else {
        return Err(Error::ConfigInvalid {
            setting: "output_dir".to_owned(),
            value: String::new(),
            detail: "set output_dir to the directory that receives dump and backup files"
                .to_owned(),
        });
    };
    let path = &dir.value;
    if !path.is_absolute() {
        return Err(Error::ConfigInvalid {
            setting: "output_dir".to_owned(),
            value: path.display().to_string(),
            detail: "output_dir must be an absolute path".to_owned(),
        });
    }
    crate::config::profile::create_private_directory(path).map_err(|error| {
        Error::ConfigUnwritable {
            path: path.clone(),
            source: error,
        }
    })?;
    if let Some(mode) = others_write_mode(path)? {
        return Err(Error::ConfigInvalid {
            setting: "output_dir".to_owned(),
            value: path.display().to_string(),
            detail: format!(
                "other users can write into the directory (mode {mode:o}); dumps go only into a directory that other users cannot change"
            ),
        });
    }
    Ok(path.clone())
}

#[cfg(unix)]
fn others_write_mode(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).map_err(|source| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source,
    })?;
    let mode = metadata.mode() & 0o777;
    Ok((mode & 0o022 != 0).then_some(mode))
}

#[cfg(not(unix))]
fn others_write_mode(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

const RESERVED_WINDOWS_STEMS: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn output_name(argument: &str, value: &str) -> Result<String> {
    let name = value.trim();
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !RESERVED_WINDOWS_STEMS.contains(&stem.as_str())
        && !name.ends_with(PARTIAL_SUFFIX)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "use a plain file name of letters, digits, dots, underscores, and hyphens that does not start or end with a dot and is not a reserved device name; it lands inside output_dir".to_owned(),
        });
    }
    Ok(name.to_owned())
}

const PARTIAL_SUFFIX: &str = ".ownpg-partial";

fn refuse_existing(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(Error::ArgumentInvalid {
            argument: "file".to_owned(),
            detail: format!(
                "{} already exists inside output_dir; choose another name, nothing is overwritten",
                path.display()
            ),
        });
    }
    Ok(())
}

fn staging_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{PARTIAL_SUFFIX}"))
}

fn prepared_file(dir: &Path, name: &str) -> Result<(PathBuf, PathBuf)> {
    let path = dir.join(name);
    refuse_existing(&path)?;
    let staging = staging_path(dir, name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&staging)
        .map_err(|error| Error::ConfigUnwritable {
            path: staging.clone(),
            source: error,
        })?;
    restrict_file(&staging)?;
    Ok((path, staging))
}

fn prepared_directory(dir: &Path, name: &str) -> Result<(PathBuf, PathBuf)> {
    let path = dir.join(name);
    refuse_existing(&path)?;
    let staging = staging_path(dir, name);
    std::fs::create_dir(&staging).map_err(|error| Error::ConfigUnwritable {
        path: staging.clone(),
        source: error,
    })?;
    restrict_directory(&staging)?;
    Ok((path, staging))
}

fn existing_input(dir: &Path, name: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| Error::ArgumentInvalid {
        argument: "file".to_owned(),
        detail: format!("{} does not exist inside output_dir", path.display()),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(Error::ArgumentInvalid {
            argument: "file".to_owned(),
            detail: format!(
                "{} is a symbolic link; restore reads only files that live inside output_dir",
                path.display()
            ),
        });
    }
    Ok(path)
}

fn discard_staging(staging: &Path, is_directory: bool) {
    let outcome = if is_directory {
        std::fs::remove_dir_all(staging)
    } else {
        std::fs::remove_file(staging)
    };
    if let Err(error) = outcome
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %staging.display(), "the partial output could not be removed");
    }
}

fn absolute_directory(argument: &str, value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() || !path.is_dir() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "an absolute path to an existing directory on this host is required".to_owned(),
        });
    }
    Ok(path)
}

fn compress_spec(value: &str) -> Result<Option<String>> {
    let spec = value.trim();
    if spec.is_empty() {
        return Ok(None);
    }
    let (method, level) = spec.split_once(':').unwrap_or((spec, ""));
    let method_ok = method.is_empty()
        || method.chars().all(|c| c.is_ascii_digit())
        || matches!(method, "gzip" | "lz4" | "zstd" | "none");
    let level_ok = level.is_empty()
        || level.chars().all(|c| c.is_ascii_digit())
        || level
            .split(',')
            .all(|part| part.chars().all(|c| c.is_ascii_alphanumeric() || c == '='));
    if !method_ok || !level_ok {
        return Err(Error::ArgumentInvalid {
            argument: "compress".to_owned(),
            detail: "use a level such as 6, or a method such as gzip, lz4, zstd, or none with an optional :level".to_owned(),
        });
    }
    Ok(Some(spec.to_owned()))
}

fn plain_text(argument: &str, value: &str, max: usize) -> Result<Option<String>> {
    let text = value.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.len() > max
        || text
            .chars()
            .any(|c| c.is_control() || matches!(c, '\'' | '"' | '\\'))
    {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!("use at most {max} printable characters without quotes"),
        });
    }
    Ok(Some(text.to_owned()))
}

fn table_patterns(call: &Call, argument: &str, names: &[String]) -> Result<Vec<String>> {
    names
        .iter()
        .map(|name| scoped_name(call, argument, name).map(|qualified| qualified.sql()))
        .collect()
}

fn command_line(program: &Path, arguments: &[String]) -> String {
    let mut parts = vec![program.display().to_string()];
    parts.extend(arguments.iter().cloned());
    parts.join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HostRun {
    pub program: String,
    pub path: String,
    pub arguments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_tail: Vec<String>,
    pub stderr_tail: Vec<String>,
    pub notice: &'static str,
}

impl HostRun {
    fn render(&self) -> String {
        let mut text = format!("{}\n{}", UNTRUSTED_NOTICE, self.path);
        for argument in &self.arguments {
            text.push(' ');
            text.push_str(argument);
        }
        text.push('\n');
        if self.dry_run {
            text.push_str("dry run: the program did not run\n");
            return text;
        }
        text.push_str(&format!(
            "exit {} after {} ms\n",
            self.exit_code
                .map_or("signal".to_owned(), |code| code.to_string()),
            self.duration_ms
        ));
        if let Some(output) = &self.output {
            text.push_str(&format!(
                "output: {output} ({} bytes)\n",
                self.output_bytes.unwrap_or(0)
            ));
        }
        for line in &self.stderr_tail {
            text.push_str("  ");
            text.push_str(line);
            text.push('\n');
        }
        text
    }
}

struct Job {
    tool: &'static str,
    program: &'static str,
    arguments: Vec<String>,
    output: Option<PathBuf>,
    staging: Option<PathBuf>,
    output_is_directory: bool,
    cwd: Option<PathBuf>,
    dry_run: bool,
    confirm: bool,
    destructive_reason: Option<String>,
    timeout_seconds: u64,
}

fn synthetic_classification(
    kind: &str,
    command: &str,
    destructive_reason: Option<String>,
) -> Classification {
    Classification {
        class: StatementClass::Maintenance,
        kind: kind.to_owned(),
        destructive_reason,
        refusals: Vec::new(),
        relations: Vec::new(),
        functions: Vec::new(),
        fingerprint: sha256_hex(command.as_bytes()).chars().take(16).collect(),
        sql_sha256: sha256_hex(command.as_bytes()),
        normalized: command.to_owned(),
        runs_outside_transaction: true,
        explain_analyze: false,
        returning: false,
    }
}

fn tail(lines: &VecDeque<String>) -> Vec<String> {
    lines.iter().cloned().collect()
}

fn push_tail(lines: &mut VecDeque<String>, line: String) {
    if lines.len() == TAIL_LINES {
        lines.pop_front();
    }
    lines.push_back(line);
}

async fn run_program(call: &Call, job: Job) -> Outcome {
    let settings = call.settings();
    let Some(path) = find_program(settings, job.program) else {
        return Err(Error::HostBinaryMissing {
            name: job.program.to_owned(),
        }
        .into());
    };
    let command = command_line(&path, &job.arguments);
    let classification =
        synthetic_classification(job.tool, &command, job.destructive_reason.clone());
    let mut facts = AuditFacts {
        operation: Some(job.tool.to_owned()),
        statement_class: Some("host".to_owned()),
        statement_hash: Some(classification.fingerprint.clone()),
        statement: Some(crate::shape::cut_graphemes(
            &command,
            crate::audit::SHORT_STATEMENT_CAP,
        )),
        ..AuditFacts::default()
    };
    let output_text = job.output.as_ref().map(|path| path.display().to_string());
    if job.dry_run {
        facts.decision = Some(Decision::DryRun);
        let run = HostRun {
            program: job.program.to_owned(),
            path: path.display().to_string(),
            arguments: job.arguments,
            output: output_text,
            output_bytes: None,
            dry_run: true,
            exit_code: None,
            duration_ms: 0,
            stdout_tail: Vec::new(),
            stderr_tail: Vec::new(),
            notice: UNTRUSTED_NOTICE,
        };
        let text = run.render();
        return Ok(ToolOutput::structured(&run, text)?.with_facts(facts).into());
    }
    let decision = match call
        .context
        .gate
        .check(call, job.tool, &classification, job.confirm)?
    {
        Verdict::Proceed(decision) => decision,
        Verdict::Ask(result) => return Ok(super::Reply::InputRequired(*result)),
    };
    facts.decision = Some(decision);
    let env = connection_env(call)
        .await
        .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
    let mut process = base_command(&path);
    process.args(&job.arguments);
    process.envs(env.vars.iter().map(|(key, value)| (key, value)));
    if let Some(cwd) = &job.cwd {
        process.current_dir(cwd);
    }
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = process.spawn().map_err(|error| {
        ToolFailure::from(Error::SubprocessFailed {
            program: job.program.to_owned(),
            status: format!("could not start: {error}"),
        })
        .with_facts(facts.clone())
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut stdout_lines = VecDeque::with_capacity(TAIL_LINES);
    let mut stderr_lines = VecDeque::with_capacity(TAIL_LINES);
    let progress = call.progress.clone();
    let mut sent = 0usize;
    let mut out_reader = stdout.map(|pipe| BufReader::new(pipe).lines());
    let mut err_reader = stderr.map(|pipe| BufReader::new(pipe).lines());
    let deadline = tokio::time::sleep(Duration::from_secs(job.timeout_seconds.clamp(1, 86_400)));
    tokio::pin!(deadline);
    let status = loop {
        tokio::select! {
            line = async {
                match out_reader.as_mut() {
                    Some(reader) => reader.next_line().await,
                    None => std::future::pending().await,
                }
            } => match line {
                Ok(Some(line)) => push_tail(&mut stdout_lines, line),
                _ => out_reader = None,
            },
            line = async {
                match err_reader.as_mut() {
                    Some(reader) => reader.next_line().await,
                    None => std::future::pending().await,
                }
            } => match line {
                Ok(Some(line)) => {
                    if let Some(progress) = &progress
                        && sent < PROGRESS_LINES
                    {
                        sent += 1;
                        progress.report(sent as f64, None, line.clone()).await;
                    }
                    push_tail(&mut stderr_lines, line);
                }
                _ => err_reader = None,
            },
            status = child.wait(), if out_reader.is_none() && err_reader.is_none() => {
                break status.map_err(|error| Error::SubprocessFailed {
                    program: job.program.to_owned(),
                    status: format!("could not be waited for: {error}"),
                });
            }
            () = call.cancel.cancelled() => {
                let _ = child.kill().await;
                break Err(Error::SubprocessFailed {
                    program: job.program.to_owned(),
                    status: "killed after the client cancelled the request".to_owned(),
                });
            }
            () = &mut deadline => {
                let _ = child.kill().await;
                break Err(Error::SubprocessFailed {
                    program: job.program.to_owned(),
                    status: format!("killed after {} seconds", job.timeout_seconds),
                });
            }
        }
    };
    drop(env);
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let succeeded = status.as_ref().is_ok_and(std::process::ExitStatus::success);
    if let Some(staging) = &job.staging {
        if succeeded {
            if let (Some(output), Err(error)) = (
                &job.output,
                std::fs::rename(staging, job.output.as_deref().unwrap_or(staging)),
            ) {
                discard_staging(staging, job.output_is_directory);
                return Err(ToolFailure::from(Error::ConfigUnwritable {
                    path: output.clone(),
                    source: error,
                })
                .with_facts(facts));
            }
        } else {
            discard_staging(staging, job.output_is_directory);
        }
    }
    let status = status.map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
    if !status.success() {
        let last = stderr_lines
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .cloned()
            .unwrap_or_default();
        return Err(ToolFailure::from(Error::SubprocessFailed {
            program: job.program.to_owned(),
            status: format!(
                "{}{}",
                status
                    .code()
                    .map_or("a signal".to_owned(), |code| format!("code {code}")),
                if last.is_empty() {
                    String::new()
                } else {
                    format!(": {last}")
                }
            ),
        })
        .with_facts(facts));
    }
    let output_bytes = job.output.as_ref().and_then(|path| {
        if job.output_is_directory {
            directory_size(path)
        } else {
            std::fs::metadata(path).ok().map(|meta| meta.len())
        }
    });
    let run = HostRun {
        program: job.program.to_owned(),
        path: path.display().to_string(),
        arguments: job.arguments,
        output: output_text,
        output_bytes,
        dry_run: false,
        exit_code: status.code(),
        duration_ms,
        stdout_tail: tail(&stdout_lines),
        stderr_tail: tail(&stderr_lines),
        notice: UNTRUSTED_NOTICE,
    };
    let text = run.render();
    Ok(ToolOutput::structured(&run, text)?.with_facts(facts).into())
}

fn directory_size(path: &Path) -> Option<u64> {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total += metadata.len();
            }
        }
    }
    Some(total)
}

const fn default_program_timeout() -> u64 {
    3_600
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DumpFormat {
    #[default]
    Plain,
    Custom,
    Directory,
    Tar,
}

impl DumpFormat {
    const fn flag(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Custom => "custom",
            Self::Directory => "directory",
            Self::Tar => "tar",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DumpArgs {
    #[schemars(
        description = "File (or directory for the directory format) name inside output_dir."
    )]
    pub file: String,
    #[serde(default)]
    pub format: DumpFormat,
    #[serde(default)]
    #[schemars(
        description = "Tables in the scoped schema to include; empty dumps the whole schema."
    )]
    pub tables: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Tables in the scoped schema to leave out.")]
    pub exclude_tables: Vec<String>,
    #[serde(default)]
    pub data_only: bool,
    #[serde(default)]
    pub schema_only: bool,
    #[serde(default)]
    pub no_owner: bool,
    #[serde(default)]
    pub no_privileges: bool,
    #[serde(default)]
    #[schemars(
        description = "Compression as a level (6) or method:level (zstd:3); empty keeps the default."
    )]
    pub compress: String,
    #[serde(default)]
    #[schemars(description = "Parallel jobs for the directory format as text; empty keeps one.")]
    pub jobs: String,
    #[serde(default = "default_program_timeout")]
    #[schemars(description = "Seconds before the program is killed (default 3600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn dump(call: Call, args: DumpArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if args.data_only && args.schema_only {
            return Err(Error::ArgumentInvalid {
                argument: "data_only".to_owned(),
                detail: "data_only and schema_only cannot both be set".to_owned(),
            }
            .into());
        }
        let dir = output_dir(&call)?;
        let name = output_name("file", &args.file)?;
        let settings = call.settings();
        let mut arguments = vec![
            format!("--dbname={}", settings.database.value),
            format!("--schema={}", settings.schema.value),
            format!("--format={}", args.format.flag()),
            "--no-password".to_owned(),
        ];
        for table in table_patterns(&call, "tables", &args.tables)? {
            arguments.push(format!("--table={table}"));
        }
        for table in table_patterns(&call, "exclude_tables", &args.exclude_tables)? {
            arguments.push(format!("--exclude-table={table}"));
        }
        if args.data_only {
            arguments.push("--data-only".to_owned());
        }
        if args.schema_only {
            arguments.push("--schema-only".to_owned());
        }
        if args.no_owner {
            arguments.push("--no-owner".to_owned());
        }
        if args.no_privileges {
            arguments.push("--no-privileges".to_owned());
        }
        if let Some(compress) = compress_spec(&args.compress)? {
            arguments.push(format!("--compress={compress}"));
        }
        if let Some(jobs) = number("jobs", &args.jobs)? {
            if args.format != DumpFormat::Directory {
                return Err(Error::ArgumentInvalid {
                    argument: "jobs".to_owned(),
                    detail: "parallel jobs need the directory format".to_owned(),
                }
                .into());
            }
            arguments.push(format!("--jobs={}", jobs.clamp(1, 64)));
        }
        if call.progress.is_some() {
            arguments.push("--verbose".to_owned());
        }
        let (output, staging) = if args.dry_run {
            (dir.join(&name), None)
        } else if args.format == DumpFormat::Directory {
            let (output, staging) = prepared_directory(&dir, &name)?;
            (output, Some(staging))
        } else {
            let (output, staging) = prepared_file(&dir, &name)?;
            (output, Some(staging))
        };
        arguments.push(format!(
            "--file={}",
            staging.as_deref().unwrap_or(&output).display()
        ));
        run_program(
            &call,
            Job {
                tool: "pg_dump",
                program: "pg_dump",
                arguments,
                output: Some(output),
                staging,
                output_is_directory: args.format == DumpFormat::Directory,
                cwd: None,
                dry_run: args.dry_run,
                confirm: true,
                destructive_reason: None,
                timeout_seconds: args.timeout_seconds,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DumpallArgs {
    #[schemars(description = "File name inside output_dir.")]
    pub file: String,
    #[serde(default)]
    #[schemars(description = "Leave password hashes out of the role definitions.")]
    pub no_role_passwords: bool,
    #[serde(default = "default_program_timeout")]
    #[schemars(description = "Seconds before the program is killed (default 3600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn dumpall_globals(call: Call, args: DumpallArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let dir = output_dir(&call)?;
        let name = output_name("file", &args.file)?;
        let settings = call.settings();
        let mut arguments = vec![
            "--globals-only".to_owned(),
            format!("--database={}", settings.database.value),
            "--no-password".to_owned(),
        ];
        if args.no_role_passwords {
            arguments.push("--no-role-passwords".to_owned());
        }
        if call.progress.is_some() {
            arguments.push("--verbose".to_owned());
        }
        let (output, staging) = if args.dry_run {
            (dir.join(&name), None)
        } else {
            let (output, staging) = prepared_file(&dir, &name)?;
            (output, Some(staging))
        };
        arguments.push(format!(
            "--file={}",
            staging.as_deref().unwrap_or(&output).display()
        ));
        run_program(
            &call,
            Job {
                tool: "pg_dumpall_globals",
                program: "pg_dumpall",
                arguments,
                output: Some(output),
                staging,
                output_is_directory: false,
                cwd: None,
                dry_run: args.dry_run,
                confirm: true,
                destructive_reason: None,
                timeout_seconds: args.timeout_seconds,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreArgs {
    #[schemars(description = "Archive file or directory name inside output_dir.")]
    pub file: String,
    #[serde(default)]
    #[schemars(description = "Tables to restore; empty restores everything in the scoped schema.")]
    pub tables: Vec<String>,
    #[serde(default)]
    pub data_only: bool,
    #[serde(default)]
    pub schema_only: bool,
    #[serde(default)]
    #[schemars(description = "Drop the objects before recreating them; destructive.")]
    pub clean: bool,
    #[serde(default)]
    pub if_exists: bool,
    #[serde(default)]
    pub no_owner: bool,
    #[serde(default)]
    pub no_privileges: bool,
    #[serde(default)]
    pub single_transaction: bool,
    #[serde(default)]
    pub exit_on_error: bool,
    #[serde(default)]
    #[schemars(description = "Parallel jobs as text; empty keeps one.")]
    pub jobs: String,
    #[serde(default = "default_program_timeout")]
    #[schemars(description = "Seconds before the program is killed (default 3600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
}

pub fn restore(call: Call, args: RestoreArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if args.data_only && args.schema_only {
            return Err(Error::ArgumentInvalid {
                argument: "data_only".to_owned(),
                detail: "data_only and schema_only cannot both be set".to_owned(),
            }
            .into());
        }
        if args.single_transaction && !args.jobs.trim().is_empty() {
            return Err(Error::ArgumentInvalid {
                argument: "jobs".to_owned(),
                detail: "parallel jobs cannot run inside a single transaction".to_owned(),
            }
            .into());
        }
        let dir = output_dir(&call)?;
        let name = output_name("file", &args.file)?;
        let input = existing_input(&dir, &name)?;
        let settings = call.settings();
        let mut arguments = vec![
            format!("--dbname={}", settings.database.value),
            format!("--schema={}", settings.schema.value),
            "--no-password".to_owned(),
        ];
        for table in &args.tables {
            let qualified = scoped_name(&call, "tables", table)?;
            arguments.push(format!("--table={}", qualified.name));
        }
        for (flag, set) in [
            ("--data-only", args.data_only),
            ("--schema-only", args.schema_only),
            ("--clean", args.clean),
            ("--if-exists", args.if_exists),
            ("--no-owner", args.no_owner),
            ("--no-privileges", args.no_privileges),
            ("--single-transaction", args.single_transaction),
            ("--exit-on-error", args.exit_on_error),
        ] {
            if set {
                arguments.push(flag.to_owned());
            }
        }
        if let Some(jobs) = number("jobs", &args.jobs)? {
            arguments.push(format!("--jobs={}", jobs.clamp(1, 64)));
        }
        if call.progress.is_some() {
            arguments.push("--verbose".to_owned());
        }
        arguments.push("--".to_owned());
        arguments.push(input.display().to_string());
        run_program(
            &call,
            Job {
                tool: "pg_restore",
                program: "pg_restore",
                arguments,
                output: None,
                staging: None,
                output_is_directory: false,
                cwd: None,
                dry_run: args.dry_run,
                confirm: args.confirm,
                destructive_reason: args.clean.then(|| {
                    format!(
                        "pg_restore --clean drops the objects in schema {} before it recreates them",
                        settings.schema.value
                    )
                }),
                timeout_seconds: args.timeout_seconds,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackupFormat {
    #[default]
    Plain,
    Tar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WalMethod {
    #[default]
    Stream,
    Fetch,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Checkpoint {
    #[default]
    Spread,
    Fast,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BasebackupArgs {
    #[schemars(description = "New directory name inside output_dir that receives the backup.")]
    pub directory: String,
    #[serde(default)]
    pub format: BackupFormat,
    #[serde(default)]
    pub wal_method: WalMethod,
    #[serde(default)]
    pub checkpoint: Checkpoint,
    #[serde(default)]
    #[schemars(description = "Backup label written into the backup; empty keeps the default.")]
    pub label: String,
    #[serde(default)]
    #[schemars(
        description = "Compression as a level (6) or method:level (zstd:3); empty keeps none."
    )]
    pub compress: String,
    #[serde(default = "default_program_timeout")]
    #[schemars(description = "Seconds before the program is killed (default 3600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn basebackup(call: Call, args: BasebackupArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let dir = output_dir(&call)?;
        let name = output_name("directory", &args.directory)?;
        let mut arguments = vec![
            format!(
                "--format={}",
                match args.format {
                    BackupFormat::Plain => "plain",
                    BackupFormat::Tar => "tar",
                }
            ),
            format!(
                "--wal-method={}",
                match args.wal_method {
                    WalMethod::Stream => "stream",
                    WalMethod::Fetch => "fetch",
                    WalMethod::None => "none",
                }
            ),
            format!(
                "--checkpoint={}",
                match args.checkpoint {
                    Checkpoint::Spread => "spread",
                    Checkpoint::Fast => "fast",
                }
            ),
            "--no-password".to_owned(),
        ];
        if let Some(label) = plain_text("label", &args.label, 100)? {
            arguments.push(format!("--label={label}"));
        }
        if let Some(compress) = compress_spec(&args.compress)? {
            arguments.push(format!("--compress={compress}"));
        }
        if call.progress.is_some() {
            arguments.push("--progress".to_owned());
            arguments.push("--verbose".to_owned());
        }
        let (output, staging) = if args.dry_run {
            (dir.join(&name), None)
        } else {
            let (output, staging) = prepared_directory(&dir, &name)?;
            (output, Some(staging))
        };
        arguments.push(format!(
            "--pgdata={}",
            staging.as_deref().unwrap_or(&output).display()
        ));
        run_program(
            &call,
            Job {
                tool: "pg_basebackup",
                program: "pg_basebackup",
                arguments,
                output: Some(output),
                staging,
                output_is_directory: true,
                cwd: None,
                dry_run: args.dry_run,
                confirm: true,
                destructive_reason: None,
                timeout_seconds: args.timeout_seconds,
            },
        )
        .await
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransferMethod {
    #[default]
    Copy,
    Link,
    Clone,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpgradeCheckArgs {
    #[schemars(description = "Absolute path of the old cluster's binary directory.")]
    pub old_bindir: String,
    #[serde(default)]
    #[schemars(
        description = "Absolute path of the new binary directory; empty uses the directory that holds pg_upgrade."
    )]
    pub new_bindir: String,
    #[schemars(description = "Absolute path of the old data directory.")]
    pub old_datadir: String,
    #[schemars(description = "Absolute path of the new, initialized data directory.")]
    pub new_datadir: String,
    #[serde(default)]
    #[schemars(description = "Port for the old cluster as text; empty keeps the default.")]
    pub old_port: String,
    #[serde(default)]
    #[schemars(description = "Port for the new cluster as text; empty keeps the default.")]
    pub new_port: String,
    #[serde(default)]
    pub method: TransferMethod,
    #[serde(default)]
    #[schemars(description = "Parallel jobs as text; empty keeps one.")]
    pub jobs: String,
    #[serde(default = "default_program_timeout")]
    #[schemars(description = "Seconds before the program is killed (default 3600).")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn upgrade_check(call: Call, args: UpgradeCheckArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let dir = output_dir(&call)?;
        let old_bindir = absolute_directory("old_bindir", &args.old_bindir)?;
        let old_datadir = absolute_directory("old_datadir", &args.old_datadir)?;
        let new_datadir = absolute_directory("new_datadir", &args.new_datadir)?;
        let new_bindir = if args.new_bindir.trim().is_empty() {
            find_program(call.settings(), "pg_upgrade")
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .ok_or_else(|| Error::HostBinaryMissing {
                    name: "pg_upgrade".to_owned(),
                })?
        } else {
            absolute_directory("new_bindir", &args.new_bindir)?
        };
        let info = call.engine().info().await;
        let mut arguments = vec![
            "--check".to_owned(),
            format!("--old-bindir={}", old_bindir.display()),
            format!("--new-bindir={}", new_bindir.display()),
            format!("--old-datadir={}", old_datadir.display()),
            format!("--new-datadir={}", new_datadir.display()),
            format!("--username={}", info.user),
        ];
        for (flag, value) in [
            ("--old-port", &args.old_port),
            ("--new-port", &args.new_port),
        ] {
            if let Some(port) = number(flag.trim_start_matches("--"), value)? {
                if !(1..=65_535).contains(&port) {
                    return Err(Error::ArgumentInvalid {
                        argument: flag.trim_start_matches("--").to_owned(),
                        detail: "a port between 1 and 65535 is required".to_owned(),
                    }
                    .into());
                }
                arguments.push(format!("{flag}={port}"));
            }
        }
        match args.method {
            TransferMethod::Copy => {}
            TransferMethod::Link => arguments.push("--link".to_owned()),
            TransferMethod::Clone => arguments.push("--clone".to_owned()),
        }
        if let Some(jobs) = number("jobs", &args.jobs)? {
            arguments.push(format!("--jobs={}", jobs.clamp(1, 64)));
        }
        if call.progress.is_some() {
            arguments.push("--verbose".to_owned());
        }
        run_program(
            &call,
            Job {
                tool: "pg_upgrade_check",
                program: "pg_upgrade",
                arguments,
                output: None,
                staging: None,
                output_is_directory: false,
                cwd: Some(dir),
                dry_run: args.dry_run,
                confirm: true,
                destructive_reason: None,
                timeout_seconds: args.timeout_seconds,
            },
        )
        .await
    })
}

pub fn routes() -> Result<Vec<Route>> {
    Ok(vec![
        route::<DumpArgs, HostRun, _>(&tool_specs::PG_DUMP, DUMP_DESCRIPTION, dump)?,
        route::<DumpallArgs, HostRun, _>(
            &tool_specs::PG_DUMPALL_GLOBALS,
            DUMPALL_DESCRIPTION,
            dumpall_globals,
        )?,
        route::<RestoreArgs, HostRun, _>(&tool_specs::PG_RESTORE, RESTORE_DESCRIPTION, restore)?,
        route::<BasebackupArgs, HostRun, _>(
            &tool_specs::PG_BASEBACKUP,
            BASEBACKUP_DESCRIPTION,
            basebackup,
        )?,
        route::<UpgradeCheckArgs, HostRun, _>(
            &tool_specs::PG_UPGRADE_CHECK,
            UPGRADE_CHECK_DESCRIPTION,
            upgrade_check,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_stay_inside_the_output_directory() {
        assert_eq!(output_name("file", " app.dump ").unwrap(), "app.dump");
        for bad in [
            "",
            ".hidden",
            "../up",
            "a/b",
            "a b",
            "a\\b",
            "..",
            "\u{0}",
            "con",
            "CON",
            "nul.dump",
            "COM1.sql",
            "trailing.",
            "x.ownpg-partial",
        ] {
            assert!(output_name("file", bad).is_err(), "{bad:?}");
        }
        assert_eq!(output_name("file", "console.dump").unwrap(), "console.dump");
        let long = "a".repeat(129);
        assert!(output_name("file", &long).is_err());
    }

    #[test]
    fn compression_specs_are_checked() {
        assert_eq!(compress_spec("").unwrap(), None);
        assert_eq!(compress_spec("6").unwrap().as_deref(), Some("6"));
        assert_eq!(compress_spec("zstd:3").unwrap().as_deref(), Some("zstd:3"));
        assert_eq!(
            compress_spec("zstd:level=3,long").unwrap().as_deref(),
            Some("zstd:level=3,long")
        );
        for bad in ["bzip2", "gzip:x;y", "6 --file=/etc/x", "zstd:3 4"] {
            assert!(compress_spec(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pgpass_values_escape_colons_and_backslashes() {
        assert_eq!(pgpass_escape("a:b\\c"), "a\\:b\\\\c");
    }

    #[test]
    fn a_missing_program_is_reported_without_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let env = crate::config::Environment::new(
            std::collections::BTreeMap::from([("OWNPG_DATABASE".to_owned(), "app".to_owned())]),
            Some(dir.path().to_path_buf()),
            None,
        );
        let paths = crate::config::AppPaths::from_base(
            dir.path().join("c"),
            dir.path().join("d"),
            dir.path().join("k"),
        );
        let (settings, _) = crate::config::resolve(
            crate::config::FlagLayer {
                pg_bindir: Some(dir.path().to_path_buf()),
                ..crate::config::FlagLayer::default()
            },
            crate::config::Sources {
                env: &env,
                paths,
                keychain: None,
            },
        )
        .unwrap();
        assert!(find_program(&settings, "pg_dump").is_none());
        assert!(output_dir_missing(&settings));
    }

    fn output_dir_missing(settings: &Settings) -> bool {
        settings.output_dir.is_none()
    }
}
