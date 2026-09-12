use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::{AuditSettings, Mode};
use crate::error::{Error, Result};

pub const FORMAT_VERSION: u32 = 1;
const CHAIN_START: &str = "chain-start";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allowed,
    Refused,
    DryRun,
    ConfirmedArgument,
    ConfirmedElicitation,
    Unparsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Local,
    OsUser,
    TokenSubject,
    Bearer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub request_id: String,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    pub mode: Mode,
    pub transport: Transport,
    pub principal: String,
    pub principal_kind: PrincipalKind,
    pub database: String,
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle_id: Option<String>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u64>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub superuser: bool,
}

#[derive(Serialize)]
struct Line<'a> {
    v: u32,
    timestamp: String,
    #[serde(flatten)]
    entry: &'a Entry,
    prev: &'a str,
}

#[derive(Serialize)]
struct Marker<'a> {
    v: u32,
    timestamp: String,
    marker: &'static str,
    process: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotated_from: Option<&'a str>,
}

pub const SHORT_STATEMENT_CAP: usize = 200;

#[derive(Debug)]
struct Open {
    file: File,
    path: PathBuf,
    written: u64,
    prev: String,
}

#[derive(Debug)]
pub struct Sink {
    inner: Mutex<Option<Open>>,
    max_bytes: u64,
    base: PathBuf,
    enabled: bool,
}

#[must_use]
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[must_use]
pub fn default_file_name(profile: Option<&str>, process: u32) -> String {
    let label = profile
        .map(|name| {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    format!("audit-{label}-{process}.jsonl")
}

impl Sink {
    pub fn disabled() -> Self {
        Self {
            inner: Mutex::new(None),
            max_bytes: 0,
            base: PathBuf::new(),
            enabled: false,
        }
    }

    pub fn open(settings: &AuditSettings, data_dir: &Path, profile: Option<&str>) -> Result<Self> {
        if !settings.enabled.value {
            return Ok(Self::disabled());
        }
        let base = settings.path.as_ref().map_or_else(
            || data_dir.join(default_file_name(profile, std::process::id())),
            |path| path.value.clone(),
        );
        let open = open_file(&base, None)?;
        Ok(Self {
            inner: Mutex::new(Some(open)),
            max_bytes: settings.max_bytes.value,
            base,
            enabled: true,
        })
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.enabled.then_some(self.base.as_path())
    }

    pub fn record(&self, entry: &Entry) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut guard = self.inner.lock().map_err(|_| Error::AuditUnwritable {
            path: self.base.clone(),
            source: std::io::Error::other("the audit lock is poisoned"),
        })?;
        let Some(open) = guard.as_mut() else {
            return Ok(());
        };
        if self.max_bytes > 0 && open.written >= self.max_bytes {
            let rotated = rotate(open)?;
            *open = open_file(&self.base, Some(&rotated))?;
        }
        let line = Line {
            v: FORMAT_VERSION,
            timestamp: now_rfc3339(),
            entry,
            prev: &open.prev,
        };
        let text = serde_json::to_string(&line).map_err(|error| Error::AuditUnwritable {
            path: open.path.clone(),
            source: std::io::Error::other(error.to_string()),
        })?;
        write_line(open, &text)
    }

    pub fn flush(&self) -> Result<()> {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(open) = guard.as_mut()
        {
            open.file.flush().map_err(|source| Error::AuditUnwritable {
                path: open.path.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

fn open_file(path: &Path, rotated_from: Option<&str>) -> Result<Open> {
    let unwritable = |source: std::io::Error| Error::AuditUnwritable {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        crate::config::profile::create_private_directory(parent).map_err(unwritable)?;
    }
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(unwritable)?;
    let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut open = Open {
        file,
        path: path.to_path_buf(),
        written,
        prev: String::new(),
    };
    let marker = Marker {
        v: FORMAT_VERSION,
        timestamp: now_rfc3339(),
        marker: CHAIN_START,
        process: std::process::id(),
        rotated_from,
    };
    let text = serde_json::to_string(&marker)
        .map_err(|error| unwritable(std::io::Error::other(error.to_string())))?;
    write_line(&mut open, &text)?;
    Ok(open)
}

fn write_line(open: &mut Open, text: &str) -> Result<()> {
    let unwritable = |source: std::io::Error| Error::AuditUnwritable {
        path: open.path.clone(),
        source,
    };
    open.file
        .write_all(text.as_bytes())
        .and_then(|()| open.file.write_all(b"\n"))
        .and_then(|()| open.file.flush())
        .map_err(unwritable)?;
    open.written += text.len() as u64 + 1;
    open.prev = sha256_hex(text.as_bytes());
    Ok(())
}

fn rotate(open: &mut Open) -> Result<String> {
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let file_name = open
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audit.jsonl");
    let rotated_name = format!("{file_name}.{stamp}");
    let rotated = open.path.with_file_name(&rotated_name);
    fs::rename(&open.path, &rotated).map_err(|source| Error::AuditUnwritable {
        path: open.path.clone(),
        source,
    })?;
    Ok(rotated_name)
}

pub fn verify_chain(path: &Path) -> std::result::Result<usize, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut prev = String::new();
    let mut count = 0usize;
    for (number, line) in text.lines().enumerate() {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| format!("line {}: {error}", number + 1))?;
        if value.get("marker").is_some() {
            prev = sha256_hex(line.as_bytes());
            continue;
        }
        let recorded = value
            .get("prev")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if recorded != prev {
            return Err(format!(
                "line {}: prev does not match the previous line",
                number + 1
            ));
        }
        prev = sha256_hex(line.as_bytes());
        count += 1;
    }
    Ok(count)
}

#[must_use]
pub fn short_statement(normalized: &str) -> Option<String> {
    (normalized.len() <= SHORT_STATEMENT_CAP).then(|| normalized.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Origin, Resolved};

    fn entry(tool: &str) -> Entry {
        Entry {
            request_id: "r1".to_owned(),
            tool: tool.to_owned(),
            operation: None,
            mode: Mode::ReadOnly,
            transport: Transport::Stdio,
            principal: "local".to_owned(),
            principal_kind: PrincipalKind::Local,
            database: "app".to_owned(),
            schema: "public".to_owned(),
            statement_class: Some("read".to_owned()),
            statement_hash: Some("abc".to_owned()),
            statement: Some("SELECT $1".to_owned()),
            decision: Decision::Allowed,
            rule: None,
            handle_id: None,
            duration_ms: 3,
            row_count: Some(1),
            truncated: false,
            outcome: None,
            superuser: false,
        }
    }

    fn settings(dir: &Path, max_bytes: u64) -> AuditSettings {
        AuditSettings {
            enabled: Resolved::preset(true),
            path: Some(Resolved::new(dir.join("audit.jsonl"), Origin::Flag)),
            max_bytes: Resolved::preset(max_bytes),
        }
    }

    #[test]
    fn every_line_chains_to_the_previous_one_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        sink.record(&entry("pg_run_query")).unwrap();
        sink.record(&entry("pg_describe")).unwrap();
        let path = sink.path().unwrap().to_path_buf();
        assert_eq!(verify_chain(&path).unwrap(), 2);
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("chain-start"));
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["v"], 1);
        assert_eq!(second["tool"], "pg_run_query");
        assert_eq!(second["decision"], "allowed");
        assert_eq!(second["principal_kind"], "local");
        assert_eq!(second["prev"], sha256_hex(lines[0].as_bytes()));
        let mut tampered = text.clone();
        tampered = tampered.replace("pg_run_query", "pg_run_write");
        fs::write(&path, tampered).unwrap();
        assert!(verify_chain(&path).unwrap_err().contains("line 3"));
    }

    #[test]
    fn rotation_moves_the_full_file_aside_and_starts_a_new_chain() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink::open(&settings(dir.path(), 300), dir.path(), None).unwrap();
        for _ in 0..4 {
            sink.record(&entry("pg_run_query")).unwrap();
        }
        let files: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(files.len() >= 2, "{files:?}");
        assert!(files.iter().any(|name| name.starts_with("audit.jsonl.")));
        assert!(verify_chain(sink.path().unwrap()).is_ok());
        let current = fs::read_to_string(sink.path().unwrap()).unwrap();
        assert!(current.lines().next().unwrap().contains("rotated_from"));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_belong_to_the_owner_only() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data");
        let sink = Sink::open(&settings(&nested, 0), &nested, None).unwrap();
        sink.record(&entry("x")).unwrap();
        assert_eq!(
            fs::metadata(sink.path().unwrap()).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::metadata(&nested).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn a_disabled_sink_writes_nothing_and_reports_no_path() {
        let sink = Sink::disabled();
        assert!(!sink.is_enabled());
        assert!(sink.path().is_none());
        sink.record(&entry("x")).unwrap();
    }

    #[test]
    fn the_default_file_name_carries_the_profile_and_the_process() {
        assert_eq!(
            default_file_name(Some("staging"), 42),
            "audit-staging-42.jsonl"
        );
        assert_eq!(
            default_file_name(Some("../x y"), 42),
            "audit-___x_y-42.jsonl"
        );
        assert_eq!(default_file_name(None, 7), "audit-default-7.jsonl");
    }

    #[test]
    fn only_a_short_normalized_statement_is_kept_in_the_clear() {
        assert!(short_statement("SELECT $1").is_some());
        assert!(short_statement(&"x".repeat(SHORT_STATEMENT_CAP + 1)).is_none());
    }
}
