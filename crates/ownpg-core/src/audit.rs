use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::{AuditFailure, AuditSettings, Mode};
use crate::error::{Error, Result};

pub const FORMAT_VERSION: u32 = 1;
const CHAIN_START: &str = "chain-start";
const GAP: &str = "gap";
const PRUNED: &str = "pruned";
const TAIL_SCAN_BYTES: u64 = 65_536;
const SECONDS_PER_DAY: u64 = 86_400;
const DEFAULT_LABEL: &str = "default";
const LABEL_DIGEST_HEX_CHARS: usize = 8;

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

impl Decision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Refused => "refused",
            Self::DryRun => "dry_run",
            Self::ConfirmedArgument => "confirmed_argument",
            Self::ConfirmedElicitation => "confirmed_elicitation",
            Self::Unparsed => "unparsed",
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<u64>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<String>,
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_id: Option<String>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub superuser: bool,
}

#[derive(Serialize)]
struct Line<'a> {
    #[serde(rename = "v")]
    format_version: u32,
    timestamp: String,
    #[serde(flatten)]
    entry: &'a Entry,
    prev: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PrunedFile {
    name: String,
    last_hash: String,
}

#[derive(Serialize)]
struct Marker<'a> {
    #[serde(rename = "v")]
    format_version: u32,
    timestamp: String,
    marker: &'static str,
    process: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    on_failure: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotated_from: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    resumed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dropped: Option<u64>,
    #[serde(skip_serializing_if = "<[PrunedFile]>::is_empty")]
    pruned: &'a [PrunedFile],
    prev: &'a str,
}

impl<'a> Marker<'a> {
    fn new(marker: &'static str, prev: &'a str) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            timestamp: now_rfc3339(),
            marker,
            process: std::process::id(),
            on_failure: None,
            rotated_from: None,
            resumed: false,
            dropped: None,
            pruned: &[],
            prev,
        }
    }
}

pub const SHORT_STATEMENT_CAP: usize = 200;

#[derive(Debug)]
struct OpenLog {
    file: File,
    path: PathBuf,
    written: u64,
    prev: String,
    rotated_to: Option<String>,
}

impl OpenLog {
    #[cfg(unix)]
    fn still_at(&self, path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        match (self.file.metadata(), fs::symlink_metadata(path)) {
            (Ok(open), Ok(named)) => open.dev() == named.dev() && open.ino() == named.ino(),
            _ => false,
        }
    }

    #[cfg(not(unix))]
    fn still_at(&self, path: &Path) -> bool {
        path.exists()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Family {
    PerProcess { prefix: String },
    Fixed { file_name: String },
}

impl Family {
    fn owns(&self, name: &str) -> bool {
        match self {
            Self::PerProcess { prefix } => name
                .strip_prefix(prefix.as_str())
                .and_then(|rest| rest.split_once(".jsonl"))
                .is_some_and(|(process, suffix)| is_digits(process) && is_rotation_suffix(suffix)),
            Self::Fixed { file_name } => name
                .strip_prefix(file_name.as_str())
                .is_some_and(|suffix| !suffix.is_empty() && is_rotation_suffix(suffix)),
        }
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_rotation_suffix(suffix: &str) -> bool {
    if suffix.is_empty() {
        return true;
    }
    let Some(rest) = suffix.strip_prefix('.') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('.').collect();
    (1..=2).contains(&parts.len()) && parts.iter().all(|part| is_digits(part))
}

#[derive(Debug)]
pub struct Sink {
    inner: Mutex<Option<OpenLog>>,
    max_bytes: u64,
    keep_files: u32,
    keep_days: u32,
    on_failure: AuditFailure,
    base: PathBuf,
    family: Family,
    enabled: bool,
    dropped: AtomicU64,
    unreported: AtomicU64,
    degraded: AtomicBool,
    last_failure: Mutex<Option<String>>,
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

fn family_label(profile: Option<&str>) -> String {
    let Some(name) = profile.filter(|name| !name.is_empty()) else {
        return DEFAULT_LABEL.to_owned();
    };
    let label: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if label == name {
        return label;
    }
    let digest: String = sha256_hex(name.as_bytes())
        .chars()
        .take(LABEL_DIGEST_HEX_CHARS)
        .collect();
    format!("{label}~{digest}")
}

#[must_use]
pub fn default_file_name(profile: Option<&str>, process: u32) -> String {
    format!("audit-{}-{process}.jsonl", family_label(profile))
}

impl Sink {
    pub fn disabled() -> Self {
        Self {
            inner: Mutex::new(None),
            max_bytes: 0,
            keep_files: 0,
            keep_days: 0,
            on_failure: AuditFailure::Continue,
            base: PathBuf::new(),
            family: Family::Fixed {
                file_name: String::new(),
            },
            enabled: false,
            dropped: AtomicU64::new(0),
            unreported: AtomicU64::new(0),
            degraded: AtomicBool::new(false),
            last_failure: Mutex::new(None),
        }
    }

    fn location(
        settings: &AuditSettings,
        data_dir: &Path,
        profile: Option<&str>,
    ) -> (PathBuf, Family) {
        match settings.path.as_ref() {
            Some(path) => {
                let file_name = path
                    .value
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (path.value.clone(), Family::Fixed { file_name })
            }
            None => (
                data_dir.join(default_file_name(profile, std::process::id())),
                Family::PerProcess {
                    prefix: format!("audit-{}-", family_label(profile)),
                },
            ),
        }
    }

    pub fn open(settings: &AuditSettings, data_dir: &Path, profile: Option<&str>) -> Result<Self> {
        if !settings.enabled.value {
            return Ok(Self::disabled());
        }
        let (base, family) = Self::location(settings, data_dir, profile);
        let on_failure = settings.on_failure.value;
        let mut open = open_file(&base, Start::Fresh { on_failure }, "")?;
        let sink = Self {
            inner: Mutex::new(None),
            max_bytes: settings.max_bytes.value,
            keep_files: settings.keep_files.value,
            keep_days: settings.keep_days.value,
            on_failure,
            base,
            family,
            enabled: true,
            dropped: AtomicU64::new(0),
            unreported: AtomicU64::new(0),
            degraded: AtomicBool::new(false),
            last_failure: Mutex::new(None),
        };
        sink.prune(&mut open);
        if let Ok(mut held) = sink.inner.lock() {
            *held = Some(open);
        }
        Ok(sink)
    }

    pub fn probe(
        settings: &AuditSettings,
        data_dir: &Path,
        profile: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        if !settings.enabled.value {
            return Ok(None);
        }
        let (base, _) = Self::location(settings, data_dir, profile);
        let unwritable = |source: std::io::Error| Error::AuditUnwritable {
            path: base.clone(),
            source,
        };
        let parent = base
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        crate::config::profile::create_private_directory(parent).map_err(unwritable)?;
        match OpenOptions::new().read(true).append(true).open(&base) {
            Ok(file) => file.try_lock().map_err(|error| match error {
                std::fs::TryLockError::WouldBlock => unwritable(std::io::Error::other(
                    "another process holds this audit file; give each server its own audit_path",
                )),
                std::fs::TryLockError::Error(source) => unwritable(source),
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tempfile::Builder::new()
                    .prefix(".ownpg-probe-")
                    .tempfile_in(parent)
                    .map_err(unwritable)?;
            }
            Err(error) => return Err(unwritable(error)),
        }
        Ok(Some(base))
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.enabled.then_some(self.base.as_path())
    }

    #[must_use]
    pub fn on_failure(&self) -> AuditFailure {
        self.on_failure
    }

    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.enabled && self.degraded.load(Ordering::Acquire)
    }

    pub fn record(&self, entry: &Entry) -> Result<()> {
        self.write(entry)
            .inspect_err(|error| self.note_failure(error))
    }

    pub fn admit(&self, read_only: bool) -> Result<()> {
        if !self.is_degraded() {
            return Ok(());
        }
        let recovered = self.with_log(|sink, open| sink.close_gap(open));
        match recovered {
            Ok(()) => Ok(()),
            Err(error) => {
                self.remember(&error);
                if self.on_failure.refuses(read_only) {
                    Err(Error::AuditDegraded {
                        policy: self.on_failure.as_str().to_owned(),
                        detail: error.to_string(),
                    })
                } else {
                    Ok(())
                }
            }
        }
    }

    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn warning(&self) -> Option<String> {
        let dropped = self.dropped();
        if dropped == 0 && !self.is_degraded() {
            return None;
        }
        let last = self
            .last_failure
            .lock()
            .ok()
            .and_then(|held| held.clone())
            .unwrap_or_else(|| "the reason was not kept".to_owned());
        let entries = if dropped == 1 { "entry" } else { "entries" };
        let state = if self.is_degraded() {
            format!(
                "it is still failing, and audit_on_failure = {} applies",
                self.on_failure
            )
        } else {
            "writing has resumed".to_owned()
        };
        Some(format!(
            "the audit log is degraded: {dropped} {entries} could not be written since this server started; {state}; the last failure was: {last}"
        ))
    }

    fn remember(&self, error: &Error) {
        if let Ok(mut held) = self.last_failure.lock() {
            *held = Some(error.to_string());
        }
    }

    fn note_failure(&self, error: &Error) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.unreported.fetch_add(1, Ordering::AcqRel);
        self.degraded.store(true, Ordering::Release);
        self.remember(error);
    }

    fn lock(&self) -> Result<MutexGuard<'_, Option<OpenLog>>> {
        self.inner.lock().map_err(|_| Error::AuditUnwritable {
            path: self.base.clone(),
            source: std::io::Error::other("the audit lock is poisoned"),
        })
    }

    fn with_log(&self, work: impl FnOnce(&Self, &mut OpenLog) -> Result<()>) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut guard = self.lock()?;
        let Some(open) = guard.as_mut() else {
            return Ok(());
        };
        self.keep_current(open)?;
        work(self, open)
    }

    fn write(&self, entry: &Entry) -> Result<()> {
        self.with_log(|sink, open| {
            sink.close_gap(open)?;
            let line = Line {
                format_version: FORMAT_VERSION,
                timestamp: now_rfc3339(),
                entry,
                prev: &open.prev,
            };
            let text = serde_json::to_string(&line).map_err(|error| Error::AuditUnwritable {
                path: open.path.clone(),
                source: std::io::Error::other(error.to_string()),
            })?;
            write_line(open, &text)
        })
    }

    fn close_gap(&self, open: &mut OpenLog) -> Result<()> {
        if !self.degraded.load(Ordering::Acquire) {
            return Ok(());
        }
        let dropped = self.unreported.load(Ordering::Acquire);
        let mut marker = Marker::new(GAP, &open.prev);
        marker.dropped = Some(dropped);
        let text = marker_text(&marker, &open.path)?;
        write_line(open, &text)?;
        self.unreported.fetch_sub(dropped, Ordering::AcqRel);
        self.degraded.store(false, Ordering::Release);
        tracing::warn!(
            dropped,
            "the audit log is writable again; a gap marker records the lost entries"
        );
        Ok(())
    }

    fn keep_current(&self, open: &mut OpenLog) -> Result<()> {
        if open.rotated_to.is_none() && !open.still_at(&self.base) {
            let fresh = open_file(
                &self.base,
                Start::Resumed {
                    on_failure: self.on_failure,
                },
                &open.prev,
            )?;
            tracing::warn!(path = %self.base.display(), "the audit file was moved or removed; a new file continues the chain");
            *open = fresh;
        }
        if self.max_bytes > 0 && open.written >= self.max_bytes {
            if open.rotated_to.is_none() {
                let rotated = rotate(open)?;
                open.rotated_to = Some(rotated);
            }
            let rotated_from = open.rotated_to.clone().unwrap_or_default();
            match open_file(
                &self.base,
                Start::Rotated {
                    on_failure: self.on_failure,
                    from: &rotated_from,
                },
                &open.prev,
            ) {
                Ok(fresh) => {
                    *open = fresh;
                    self.prune(open);
                }
                Err(error) => {
                    tracing::warn!(%error, "the audit log could not be reopened after rotation; still writing to the rotated file");
                }
            }
        }
        Ok(())
    }

    fn prune(&self, open: &mut OpenLog) {
        if self.keep_days == 0 && self.keep_files == 0 {
            return;
        }
        let parent = self
            .base
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let active: Vec<String> = [self.base.as_path(), open.path.as_path()]
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let entries = match fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(%error, "the audit directory could not be listed for pruning");
                return;
            }
        };
        let mut closed: Vec<(SystemTime, String)> = entries
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_owned();
                if active.contains(&name) || !self.family.owns(&name) {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                metadata
                    .file_type()
                    .is_file()
                    .then(|| metadata.modified().ok())
                    .flatten()
                    .map(|modified| (modified, name))
            })
            .collect();
        closed.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| rotation_order(&left.1).cmp(&rotation_order(&right.1)))
        });
        let cutoff = (self.keep_days > 0)
            .then(|| {
                SystemTime::now().checked_sub(Duration::from_secs(
                    u64::from(self.keep_days).saturating_mul(SECONDS_PER_DAY),
                ))
            })
            .flatten();
        let over_count = if self.keep_files > 0 {
            closed.len().saturating_sub(self.keep_files as usize)
        } else {
            0
        };
        let mut pruned = Vec::new();
        for (position, (modified, name)) in closed.iter().enumerate() {
            let expired = cutoff.is_some_and(|cutoff| *modified < cutoff);
            if !expired && position >= over_count {
                continue;
            }
            match remove_closed(&parent.join(name)) {
                Ok(Some(last_hash)) => pruned.push(PrunedFile {
                    name: name.clone(),
                    last_hash,
                }),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, file = %name, "an old audit file could not be removed");
                }
            }
        }
        if pruned.is_empty() {
            return;
        }
        let mut marker = Marker::new(PRUNED, &open.prev);
        marker.pruned = &pruned;
        let written = marker_text(&marker, &open.path).and_then(|text| write_line(open, &text));
        if let Err(error) = written {
            tracing::warn!(%error, "the pruned audit files could not be recorded in the chain");
        }
    }

    pub fn flush(&self) -> Result<()> {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(open) = guard.as_mut()
        {
            open.file
                .flush()
                .and_then(|()| open.file.sync_data())
                .map_err(|source| Error::AuditUnwritable {
                    path: open.path.clone(),
                    source,
                })?;
        }
        Ok(())
    }
}

fn rotation_order(name: &str) -> Vec<u64> {
    name.rsplit(".jsonl")
        .next()
        .unwrap_or_default()
        .split('.')
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn remove_closed(path: &Path) -> std::io::Result<Option<String>> {
    let mut file = File::open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
        Err(std::fs::TryLockError::Error(error)) => return Err(error),
    }
    let last_hash = last_line_hash(&mut file)?;
    drop(file);
    fs::remove_file(path)?;
    Ok(Some(last_hash))
}

fn last_line_hash(file: &mut File) -> std::io::Result<String> {
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(String::new());
    }
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL_SCAN_BYTES)))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    if tail.last() == Some(&b'\n') {
        tail.pop();
    }
    let last_line = tail
        .rsplit(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    Ok(sha256_hex(last_line))
}

#[derive(Debug, Clone, Copy)]
enum Start<'a> {
    Fresh {
        on_failure: AuditFailure,
    },
    Resumed {
        on_failure: AuditFailure,
    },
    Rotated {
        on_failure: AuditFailure,
        from: &'a str,
    },
}

fn marker_text(marker: &Marker<'_>, path: &Path) -> Result<String> {
    serde_json::to_string(marker).map_err(|error| Error::AuditUnwritable {
        path: path.to_path_buf(),
        source: std::io::Error::other(error.to_string()),
    })
}

fn open_file(path: &Path, start: Start<'_>, carried: &str) -> Result<OpenLog> {
    let unwritable = |source: std::io::Error| Error::AuditUnwritable {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        crate::config::profile::create_private_directory(parent).map_err(unwritable)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(unwritable)?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => unwritable(std::io::Error::other(
            "another process holds this audit file; give each server its own audit_path",
        )),
        std::fs::TryLockError::Error(source) => unwritable(source),
    })?;
    let (written, tail) = recover_tail(&mut file).map_err(unwritable)?;
    let starts_file = written == 0;
    let prev = if starts_file {
        carried.to_owned()
    } else {
        tail
    };
    let mut open = OpenLog {
        file,
        path: path.to_path_buf(),
        written,
        prev,
        rotated_to: None,
    };
    let mut marker = Marker::new(CHAIN_START, &open.prev);
    match start {
        Start::Fresh { on_failure } => marker.on_failure = Some(on_failure.as_str()),
        Start::Resumed { on_failure } => {
            marker.on_failure = Some(on_failure.as_str());
            marker.resumed = starts_file && !carried.is_empty();
        }
        Start::Rotated { on_failure, from } => {
            marker.on_failure = Some(on_failure.as_str());
            marker.rotated_from = Some(from);
        }
    }
    if !starts_file {
        marker.rotated_from = None;
        marker.resumed = false;
    }
    let text = marker_text(&marker, path)?;
    write_line(&mut open, &text)?;
    Ok(open)
}

fn recover_tail(file: &mut File) -> std::io::Result<(u64, String)> {
    let mut written = file.metadata()?.len();
    if written == 0 {
        return Ok((0, String::new()));
    }
    let start = written.saturating_sub(TAIL_SCAN_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    if tail.last() != Some(&b'\n') {
        file.write_all(b"\n")?;
        written += 1;
        let torn = tail
            .rsplit(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        return Ok((written, sha256_hex(torn)));
    }
    tail.pop();
    let last_line = tail
        .rsplit(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    Ok((written, sha256_hex(last_line)))
}

fn write_line(open: &mut OpenLog, text: &str) -> Result<()> {
    let mut bytes = Vec::with_capacity(text.len() + 1);
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(b'\n');
    if let Err(source) = open.file.write_all(&bytes).and_then(|()| open.file.flush()) {
        if let Err(error) = open.file.set_len(open.written) {
            tracing::warn!(%error, "a partly written audit line could not be cut back");
        }
        return Err(Error::AuditUnwritable {
            path: open.path.clone(),
            source,
        });
    }
    open.written += bytes.len() as u64;
    open.prev = sha256_hex(text.as_bytes());
    Ok(())
}

fn rotate(open: &mut OpenLog) -> Result<String> {
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let file_name = open
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audit.jsonl")
        .to_owned();
    let mut rotated_name = format!("{file_name}.{stamp}");
    let mut counter = 1u32;
    while open.path.with_file_name(&rotated_name).exists() {
        rotated_name = format!("{file_name}.{stamp}.{counter}");
        counter += 1;
    }
    let rotated = open.path.with_file_name(&rotated_name);
    fs::rename(&open.path, &rotated).map_err(|source| Error::AuditUnwritable {
        path: open.path.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(&rotated, fs::Permissions::from_mode(0o400)) {
            tracing::warn!(%error, file = %rotated_name, "a rotated audit file could not be made read-only");
        }
    }
    open.path = rotated;
    Ok(rotated_name)
}

pub fn verify_chain(path: &Path) -> std::result::Result<usize, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut prev = String::new();
    let mut count = 0usize;
    let mut number = 0usize;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        number += 1;
        if line.pop() != Some(b'\n') {
            return Err(format!(
                "line {number}: the line has no newline, so it was cut off while being written"
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&line).map_err(|error| format!("line {number}: {error}"))?;
        let version = value
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("line {number}: the format version `v` is missing"))?;
        if version > u64::from(FORMAT_VERSION) {
            return Err(format!(
                "line {number}: format version {version} is newer than this OwnPG reads ({FORMAT_VERSION}); check the file with a newer ownpg"
            ));
        }
        let recorded = value
            .get("prev")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("line {number}: `prev` is missing"))?;
        let marker = value.get("marker").and_then(serde_json::Value::as_str);
        let starts_file = marker == Some(CHAIN_START)
            && (value.get("rotated_from").is_some()
                || value.get("resumed").and_then(serde_json::Value::as_bool) == Some(true));
        if number == 1 {
            if marker != Some(CHAIN_START) {
                return Err(
                    "line 1: the file does not open with a chain-start marker, so its head may be missing"
                        .to_owned(),
                );
            }
            if !recorded.is_empty() && !starts_file {
                return Err(
                    "line 1: the chain-start marker links to a line that is not in this file, so its head may be missing"
                        .to_owned(),
                );
            }
        } else if starts_file {
            return Err(format!(
                "line {number}: a marker that opens a new file appears in the middle of this one"
            ));
        } else if recorded != prev {
            return Err(match marker {
                Some(_) => {
                    format!("line {number}: the chain marker does not link to the previous line")
                }
                None => format!("line {number}: prev does not match the previous line"),
            });
        }
        prev = sha256_hex(&line);
        if marker.is_none() {
            count += 1;
        }
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
            call_id: Some(7),
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
            relations: vec!["public.notes".to_owned()],
            decision: Decision::Allowed,
            rule: None,
            handle_id: None,
            cursor_id: None,
            duration_ms: 3,
            row_count: Some(1),
            rows_affected: None,
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
            keep_files: Resolved::preset(0),
            keep_days: Resolved::preset(0),
            on_failure: Resolved::preset(AuditFailure::RefuseWrites),
        }
    }

    fn per_process(dir: &Path, keep_days: u32) -> AuditSettings {
        AuditSettings {
            path: None,
            keep_days: Resolved::preset(keep_days),
            ..settings(dir, 0)
        }
    }

    fn age(path: &Path, days: u64) {
        let file = OpenOptions::new().append(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * SECONDS_PER_DAY))
            .unwrap();
    }

    fn lines(path: &Path) -> Vec<serde_json::Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[cfg(unix)]
    fn break_directory(nested: &Path) {
        fs::remove_dir_all(nested).unwrap();
        fs::write(nested, "").unwrap();
    }

    #[cfg(unix)]
    fn repair_directory(nested: &Path) {
        fs::remove_file(nested).unwrap();
    }

    #[test]
    fn the_chain_links_across_restarts_and_a_spliced_marker_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        {
            let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
            sink.record(&entry("pg_run_query")).unwrap();
        }
        let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        sink.record(&entry("pg_describe")).unwrap();
        drop(sink);
        assert_eq!(verify_chain(&path).unwrap(), 2);
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let second_marker: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(second_marker["prev"], sha256_hex(lines[1].as_bytes()));
        let spliced = format!(
            "{}\n{}\n{{\"v\":1,\"marker\":\"chain-start\",\"prev\":\"\"}}\n{}\n",
            lines[0], lines[1], lines[3]
        );
        fs::write(&path, spliced).unwrap();
        assert!(verify_chain(&path).unwrap_err().contains("line 3"));
    }

    #[test]
    fn a_torn_last_line_is_terminated_before_the_next_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        {
            let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
            sink.record(&entry("pg_run_query")).unwrap();
        }
        let mut torn = fs::read_to_string(&path).unwrap();
        torn.push_str("{\"v\":1,\"tool\":\"cut");
        fs::write(&path, torn).unwrap();
        let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        sink.record(&entry("pg_describe")).unwrap();
        drop(sink);
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[2].ends_with("cut"));
        assert!(lines[3].contains("chain-start"));
        assert!(verify_chain(&path).unwrap_err().contains("line 3"));
    }

    #[test]
    fn a_second_writer_on_the_same_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let first = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        let second = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap_err();
        assert_eq!(second.id().as_str(), "audit.unwritable");
        drop(first);
        Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
    }

    #[test]
    fn rotated_files_are_pruned_to_the_keep_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = settings(dir.path(), 300);
        settings.keep_files = Resolved::preset(1);
        let sink = Sink::open(&settings, dir.path(), None).unwrap();
        for _ in 0..12 {
            sink.record(&entry("pg_run_query")).unwrap();
        }
        let rotated = fs::read_dir(dir.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("audit.jsonl.")
            })
            .count();
        assert_eq!(rotated, 1);
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
        let keys: Vec<&str> = second
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for expected in [
            "v",
            "request_id",
            "timestamp",
            "tool",
            "mode",
            "transport",
            "principal",
            "principal_kind",
            "database",
            "schema",
            "decision",
            "duration_ms",
            "truncated",
            "superuser",
            "prev",
        ] {
            assert!(keys.contains(&expected), "{expected} missing from {keys:?}");
        }
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
        assert_eq!(sink.dropped(), 0);
        assert!(sink.warning().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_that_cannot_be_written_is_counted_and_surfaced_as_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data");
        let sink = Sink::open(&settings(&nested, 1), &nested, None).unwrap();
        sink.record(&entry("pg_run_query")).unwrap();
        assert_eq!(sink.dropped(), 0);
        assert!(sink.warning().is_none());
        break_directory(&nested);
        for expected in 1..=2 {
            let error = sink.record(&entry("pg_run_query")).unwrap_err();
            assert_eq!(error.id().as_str(), "audit.unwritable");
            assert_eq!(sink.dropped(), expected);
        }
        let warning = sink.warning().unwrap();
        assert!(warning.contains("2 entries"), "{warning}");
        assert!(warning.contains("the audit log is degraded"), "{warning}");
    }

    #[test]
    fn the_default_file_name_carries_the_profile_and_the_process() {
        assert_eq!(
            default_file_name(Some("staging"), 42),
            "audit-staging-42.jsonl"
        );
        let unsafe_name = default_file_name(Some("../x y"), 42);
        assert!(unsafe_name.starts_with("audit-___x_y~"), "{unsafe_name}");
        assert!(unsafe_name.ends_with("-42.jsonl"), "{unsafe_name}");
        assert_ne!(unsafe_name, default_file_name(Some("__.x_y"), 42));
        assert_ne!(
            default_file_name(Some("a b"), 1),
            default_file_name(Some("a_b"), 1)
        );
        assert_eq!(default_file_name(None, 7), "audit-default-7.jsonl");
        assert_eq!(default_file_name(Some(""), 7), "audit-default-7.jsonl");
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_log_refuses_writes_until_it_recovers_and_a_gap_marker_counts_the_loss() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data");
        let sink = Sink::open(&settings(&nested, 0), &nested, None).unwrap();
        sink.admit(false).unwrap();
        break_directory(&nested);
        sink.record(&entry("pg_run_write")).unwrap_err();
        sink.record(&entry("pg_run_write")).unwrap_err();
        assert!(sink.is_degraded());
        let refused = sink.admit(false).unwrap_err();
        assert_eq!(refused.id().as_str(), "audit.degraded");
        assert!(refused.to_string().contains("refuse-writes"), "{refused}");
        sink.admit(true).unwrap();
        assert!(sink.warning().unwrap().contains("still failing"));
        repair_directory(&nested);
        sink.admit(false).unwrap();
        assert!(!sink.is_degraded());
        assert!(sink.warning().unwrap().contains("writing has resumed"));
        sink.record(&entry("pg_run_write")).unwrap();
        let path = nested.join("audit.jsonl");
        let written = lines(&path);
        let gap = written.iter().find(|line| line["marker"] == "gap").unwrap();
        assert_eq!(gap["dropped"], 2);
        assert_eq!(written.first().unwrap()["resumed"], true);
        assert_eq!(verify_chain(&path).unwrap(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn refuse_all_blocks_reads_and_continue_blocks_nothing() {
        for (policy, read_refused, write_refused) in [
            (AuditFailure::RefuseAll, true, true),
            (AuditFailure::Continue, false, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let nested = dir.path().join("data");
            let mut chosen = settings(&nested, 0);
            chosen.on_failure = Resolved::preset(policy);
            let sink = Sink::open(&chosen, &nested, None).unwrap();
            break_directory(&nested);
            sink.record(&entry("pg_run_query")).unwrap_err();
            assert_eq!(sink.admit(true).is_err(), read_refused, "{policy}");
            assert_eq!(sink.admit(false).is_err(), write_refused, "{policy}");
        }
    }

    #[test]
    fn the_first_line_must_open_the_chain_so_a_cut_head_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        sink.record(&entry("pg_run_query")).unwrap();
        sink.record(&entry("pg_describe")).unwrap();
        let path = sink.path().unwrap().to_path_buf();
        drop(sink);
        let text = fs::read_to_string(&path).unwrap();
        let cut: String = text
            .lines()
            .skip(1)
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(&path, cut).unwrap();
        assert!(verify_chain(&path).unwrap_err().starts_with("line 1:"));
        let later_marker: String = text
            .lines()
            .skip(1)
            .take(1)
            .map(|line| line.replace("\"tool\"", "\"marker\":\"chain-start\",\"tool\""))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(&path, later_marker).unwrap();
        assert!(verify_chain(&path).unwrap_err().contains("line 1"));
    }

    #[test]
    fn a_newer_format_or_a_missing_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        fs::write(
            &path,
            "{\"v\":9,\"marker\":\"chain-start\",\"prev\":\"\"}\n",
        )
        .unwrap();
        assert!(verify_chain(&path).unwrap_err().contains("newer"));
        fs::write(&path, "{\"marker\":\"chain-start\",\"prev\":\"\"}\n").unwrap();
        assert!(verify_chain(&path).unwrap_err().contains("`v` is missing"));
        fs::write(&path, "{\"v\":1,\"marker\":\"chain-start\",\"prev\":\"\"}").unwrap();
        assert!(verify_chain(&path).unwrap_err().contains("no newline"));
    }

    #[test]
    fn a_rotated_file_links_to_the_file_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink::open(&settings(dir.path(), 300), dir.path(), None).unwrap();
        for _ in 0..3 {
            sink.record(&entry("pg_run_query")).unwrap();
        }
        let current = sink.path().unwrap().to_path_buf();
        let first = lines(&current).into_iter().next().unwrap();
        let rotated_name = first["rotated_from"].as_str().unwrap().to_owned();
        let rotated = dir.path().join(&rotated_name);
        let rotated_text = fs::read_to_string(&rotated).unwrap();
        let last = rotated_text.lines().last().unwrap();
        assert_eq!(first["prev"], sha256_hex(last.as_bytes()));
        assert!(verify_chain(&rotated).is_ok());
        assert!(verify_chain(&current).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&rotated).unwrap().mode() & 0o777, 0o400);
        }
    }

    #[test]
    fn old_files_of_this_profile_are_pruned_and_recorded_but_live_and_foreign_files_stay() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("audit-staging-11.jsonl");
        let old_rotated = dir.path().join("audit-staging-11.jsonl.1700000000");
        let recent = dir.path().join("audit-staging-12.jsonl");
        let live = dir.path().join("audit-staging-13.jsonl");
        let foreign = dir.path().join("audit-staging-x-14.jsonl");
        let unrelated = dir.path().join("audit-staging-15.jsonl.bak");
        for path in [&old, &old_rotated, &recent, &live, &foreign, &unrelated] {
            fs::write(path, "{\"v\":1,\"marker\":\"chain-start\",\"prev\":\"\"}\n").unwrap();
            age(path, 400);
        }
        age(&recent, 3);
        let holder = OpenOptions::new().append(true).open(&live).unwrap();
        holder.try_lock().unwrap();
        let sink = Sink::open(&per_process(dir.path(), 366), dir.path(), Some("staging")).unwrap();
        assert!(!old.exists());
        assert!(!old_rotated.exists());
        for kept in [&recent, &live, &foreign, &unrelated] {
            assert!(kept.exists(), "{}", kept.display());
        }
        let written = lines(sink.path().unwrap());
        let pruned = written
            .iter()
            .find(|line| line["marker"] == "pruned")
            .unwrap();
        let names: Vec<&str> = pruned["pruned"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "audit-staging-11.jsonl",
                "audit-staging-11.jsonl.1700000000"
            ]
        );
        assert_eq!(
            pruned["pruned"][0]["last_hash"],
            sha256_hex(b"{\"v\":1,\"marker\":\"chain-start\",\"prev\":\"\"}")
        );
        assert!(verify_chain(sink.path().unwrap()).is_ok());
        drop(holder);
    }

    #[test]
    fn a_count_cap_keeps_only_the_newest_closed_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut chosen = per_process(dir.path(), 0);
        chosen.keep_files = Resolved::preset(1);
        for (process, days) in [(21, 30), (22, 20), (23, 10)] {
            let path = dir.path().join(format!("audit-default-{process}.jsonl"));
            fs::write(
                &path,
                "{\"v\":1,\"marker\":\"chain-start\",\"prev\":\"\"}\n",
            )
            .unwrap();
            age(&path, days);
        }
        let _sink = Sink::open(&chosen, dir.path(), None).unwrap();
        assert!(!dir.path().join("audit-default-21.jsonl").exists());
        assert!(!dir.path().join("audit-default-22.jsonl").exists());
        assert!(dir.path().join("audit-default-23.jsonl").exists());
    }

    #[test]
    fn keep_days_zero_and_keep_files_zero_prune_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("audit-default-31.jsonl");
        fs::write(&old, "{\"v\":1,\"marker\":\"chain-start\",\"prev\":\"\"}\n").unwrap();
        age(&old, 4000);
        let _sink = Sink::open(&per_process(dir.path(), 0), dir.path(), None).unwrap();
        assert!(old.exists());
    }

    #[test]
    fn a_removed_log_file_is_replaced_and_the_chain_continues() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        sink.record(&entry("pg_run_query")).unwrap();
        let path = sink.path().unwrap().to_path_buf();
        let before = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        sink.record(&entry("pg_describe")).unwrap();
        let after = lines(&path);
        assert_eq!(after.first().unwrap()["resumed"], true);
        assert_eq!(
            after.first().unwrap()["prev"],
            sha256_hex(before.lines().last().unwrap().as_bytes())
        );
        assert_eq!(verify_chain(&path).unwrap(), 1);
    }

    #[test]
    fn the_probe_checks_the_directory_without_leaving_an_audit_file() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = settings(dir.path(), 0);
        let probed = Sink::probe(&chosen, dir.path(), None).unwrap().unwrap();
        assert!(!probed.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        let held = Sink::open(&chosen, dir.path(), None).unwrap();
        let refused = Sink::probe(&chosen, dir.path(), None).unwrap_err();
        assert_eq!(refused.id().as_str(), "audit.unwritable");
        drop(held);
        Sink::probe(&chosen, dir.path(), None).unwrap();
    }

    #[test]
    fn the_chain_start_records_the_failure_policy() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Sink::open(&settings(dir.path(), 0), dir.path(), None).unwrap();
        let first = lines(sink.path().unwrap()).into_iter().next().unwrap();
        assert_eq!(first["on_failure"], "refuse-writes");
        sink.record(&entry("pg_run_query")).unwrap();
        let second = lines(sink.path().unwrap()).into_iter().nth(1).unwrap();
        assert_eq!(second["relations"][0], "public.notes");
        assert_eq!(second["call_id"], 7);
    }

    #[test]
    fn only_a_short_normalized_statement_is_kept_in_the_clear() {
        assert!(short_statement("SELECT $1").is_some());
        assert!(short_statement(&"x".repeat(SHORT_STATEMENT_CAP + 1)).is_none());
    }
}
