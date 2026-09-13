pub mod describe;
pub mod environment;
pub mod http;
pub mod libpq;
pub mod presets;
pub mod profile;
mod resolve;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub use environment::Environment;
pub use http::{
    AuthMode, AuthSettings, HttpEntry, HttpFlags, HttpSettings, MCP_PATH, OauthSettings,
};
pub use resolve::{
    FlagLayer, KeychainLookup, Sources, SshTarget, Warning, ci_no_input, parse_ssh_target,
    parse_tool_groups, resolve,
};

pub const FILE_CAP_BYTES: u64 = 1_048_576;
pub const PROFILE_FORMAT: u32 = 1;
pub const DEFAULT_SCHEMA: &str = "public";
pub const DEFAULT_PORT: u16 = 5432;
pub const DEFAULT_ROW_CAP: u32 = 100;
pub const MAX_ROW_CAP: u32 = 1_000;
pub const DEFAULT_BYTE_CAP: u32 = 262_144;
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(300);
pub const DEFAULT_HANDLE_EXPIRY: Duration = Duration::from_secs(60);
pub const DEFAULT_CURSOR_EXPIRY: Duration = Duration::from_secs(30);

#[must_use]
pub fn keychain_account(profile: &str) -> String {
    format!("profile:{profile}")
}

#[must_use]
pub fn ssh_keychain_account(profile: &str) -> String {
    format!("ssh:{profile}")
}
pub const DEFAULT_AUDIT_MAX_BYTES: u64 = 52_428_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl Mode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WriteOnly => "write-only",
            Self::ReadWrite => "read-write",
        }
    }

    #[must_use]
    pub const fn allows_reads(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    #[must_use]
    pub const fn allows_writes(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "read-only" | "readonly" | "ro" => Some(Self::ReadOnly),
            "write-only" | "writeonly" | "wo" => Some(Self::WriteOnly),
            "read-write" | "readwrite" | "rw" => Some(Self::ReadWrite),
            _ => None,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Allow => "allow",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "disable" => Some(Self::Disable),
            "allow" => Some(Self::Allow),
            "prefer" => Some(Self::Prefer),
            "require" => Some(Self::Require),
            "verify-ca" => Some(Self::VerifyCa),
            "verify-full" => Some(Self::VerifyFull),
            _ => None,
        }
    }
}

impl fmt::Display for SslMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelBinding {
    Disable,
    Prefer,
    Require,
}

impl ChannelBinding {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "disable" => Some(Self::Disable),
            "prefer" => Some(Self::Prefer),
            "require" => Some(Self::Require),
            _ => None,
        }
    }
}

impl fmt::Display for ChannelBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SshTransport {
    InProcess,
    System,
}

impl SshTransport {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProcess => "in-process",
            Self::System => "system",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "in-process" | "inprocess" | "russh" => Some(Self::InProcess),
            "system" | "openssh" => Some(Self::System),
            _ => None,
        }
    }
}

impl fmt::Display for SshTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ToolGroup {
    Write,
    Transactions,
    Ddl,
    Roles,
    Maintenance,
    Monitoring,
    Host,
}

impl ToolGroup {
    pub const ALL: [Self; 7] = [
        Self::Write,
        Self::Transactions,
        Self::Ddl,
        Self::Roles,
        Self::Maintenance,
        Self::Monitoring,
        Self::Host,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Transactions => "transactions",
            Self::Ddl => "ddl",
            Self::Roles => "roles",
            Self::Maintenance => "maintenance",
            Self::Monitoring => "monitoring",
            Self::Host => "host",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "write" => Some(Self::Write),
            "transactions" => Some(Self::Transactions),
            "ddl" => Some(Self::Ddl),
            "roles" => Some(Self::Roles),
            "maintenance" => Some(Self::Maintenance),
            "monitoring" => Some(Self::Monitoring),
            "host" => Some(Self::Host),
            _ => None,
        }
    }

    #[must_use]
    pub const fn loads_by_mode(self, mode: Mode) -> bool {
        matches!(self, Self::Write | Self::Transactions) && mode.allows_writes()
    }

    #[must_use]
    pub const fn allowed_in(self, mode: Mode) -> bool {
        match self {
            Self::Write | Self::Transactions | Self::Ddl | Self::Roles | Self::Maintenance => {
                mode.allows_writes()
            }
            Self::Monitoring => mode.allows_reads(),
            Self::Host => true,
        }
    }
}

impl fmt::Display for ToolGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    Flag,
    Environment,
    Profile,
    Libpq,
    Preset,
}

impl Origin {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Environment => "environment",
            Self::Profile => "profile",
            Self::Libpq => "libpq",
            Self::Preset => "built-in default",
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<T> {
    pub value: T,
    pub origin: Origin,
}

impl<T> Resolved<T> {
    #[must_use]
    pub const fn new(value: T, origin: Origin) -> Self {
        Self { value, origin }
    }

    #[must_use]
    pub const fn preset(value: T) -> Self {
        Self {
            value,
            origin: Origin::Preset,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Secret(SecretString);

impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(SecretString::from(value))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        use secrecy::ExposeSecret;
        self.0.expose_secret()
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl Eq for Secret {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub dsn: Option<Resolved<Secret>>,
    pub service: Option<Resolved<String>>,
    pub host: Option<Resolved<String>>,
    pub hostaddr: Option<Resolved<String>>,
    pub port: Resolved<u16>,
    pub user: Resolved<String>,
    pub password: Option<Resolved<Secret>>,
    pub sslmode: Resolved<SslMode>,
    pub sslrootcert: Option<Resolved<PathBuf>>,
    pub sslcert: Option<Resolved<PathBuf>>,
    pub sslkey: Option<Resolved<PathBuf>>,
    pub channel_binding: Resolved<ChannelBinding>,
    pub connect_timeout: Resolved<Duration>,
    pub application_name: Resolved<String>,
    pub options: Option<Resolved<String>>,
    pub pooled: Resolved<Option<bool>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitSettings {
    pub statement_timeout: Resolved<Duration>,
    pub lock_timeout: Resolved<Duration>,
    pub transaction_timeout: Resolved<Duration>,
    pub handle_expiry: Resolved<Duration>,
    pub cursor_expiry: Resolved<Duration>,
    pub row_cap: Resolved<u32>,
    pub byte_cap: Resolved<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSettings {
    pub host: Resolved<String>,
    pub port: Resolved<u16>,
    pub user: Option<Resolved<String>>,
    pub key_file: Option<Resolved<PathBuf>>,
    pub agent: Resolved<bool>,
    pub password: Option<Resolved<Secret>>,
    pub trust_new_host: Resolved<bool>,
    pub transport: Resolved<SshTransport>,
    pub jump: Resolved<Vec<String>>,
    pub known_hosts: Option<Resolved<PathBuf>>,
    pub config_file: Option<Resolved<PathBuf>>,
    pub connect_timeout: Resolved<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSettings {
    pub enabled: Resolved<bool>,
    pub path: Option<Resolved<PathBuf>>,
    pub max_bytes: Resolved<u64>,
    pub keep_files: Resolved<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub config_file: PathBuf,
}

impl AppPaths {
    #[must_use]
    pub fn from_base(config_dir: PathBuf, data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        let config_file = config_dir.join("profiles.toml");
        let log_dir = data_dir.join("logs");
        Self {
            config_dir,
            data_dir,
            cache_dir,
            log_dir,
            config_file,
        }
    }

    #[must_use]
    pub fn with_config_file(mut self, config_file: PathBuf) -> Self {
        self.config_file = config_file;
        self
    }

    #[must_use]
    pub fn with_log_dir(mut self, log_dir: PathBuf) -> Self {
        self.log_dir = log_dir;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub profile: Option<String>,
    pub mode: Resolved<Mode>,
    pub database: Resolved<String>,
    pub schema: Resolved<String>,
    pub connection: ConnectionSettings,
    pub limits: LimitSettings,
    pub strict_role: Resolved<bool>,
    pub tools: Resolved<Vec<ToolGroup>>,
    pub ssh: Option<SshSettings>,
    pub audit: AuditSettings,
    pub pg_bindir: Option<Resolved<PathBuf>>,
    pub output_dir: Option<Resolved<PathBuf>>,
    pub no_input: Resolved<bool>,
    pub http: HttpSettings,
    pub paths: AppPaths,
}

impl Settings {
    #[must_use]
    pub fn loaded_groups(&self) -> Vec<ToolGroup> {
        let mode = self.mode.value;
        let mut groups: Vec<ToolGroup> = ToolGroup::ALL
            .into_iter()
            .filter(|group| group.loads_by_mode(mode))
            .collect();
        for group in &self.tools.value {
            if group.allowed_in(mode) && !groups.contains(group) {
                groups.push(*group);
            }
        }
        groups.sort_unstable();
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mode_parses_its_documented_spellings() {
        assert_eq!(Mode::parse("read-only"), Some(Mode::ReadOnly));
        assert_eq!(Mode::parse("rw"), Some(Mode::ReadWrite));
        assert_eq!(Mode::parse("write-only"), Some(Mode::WriteOnly));
        assert_eq!(Mode::parse("all"), None);
        assert!(Mode::ReadOnly.allows_reads());
        assert!(!Mode::ReadOnly.allows_writes());
        assert!(!Mode::WriteOnly.allows_reads());
        assert!(Mode::ReadWrite.allows_writes());
    }

    #[test]
    fn write_groups_load_by_mode_and_the_rest_need_a_name() {
        assert!(ToolGroup::Write.loads_by_mode(Mode::ReadWrite));
        assert!(ToolGroup::Transactions.loads_by_mode(Mode::WriteOnly));
        assert!(!ToolGroup::Write.loads_by_mode(Mode::ReadOnly));
        assert!(!ToolGroup::Ddl.loads_by_mode(Mode::ReadWrite));
        assert!(ToolGroup::Ddl.allowed_in(Mode::ReadWrite));
        assert!(!ToolGroup::Ddl.allowed_in(Mode::ReadOnly));
        assert!(ToolGroup::Monitoring.allowed_in(Mode::ReadOnly));
        assert!(!ToolGroup::Monitoring.allowed_in(Mode::WriteOnly));
        assert!(ToolGroup::Host.allowed_in(Mode::ReadOnly));
    }

    #[test]
    fn a_secret_never_prints_its_value() {
        let secret = Secret::new("hunter2".to_owned());
        let shown = format!("{secret:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn the_config_file_sits_in_the_config_dir_unless_overridden() {
        let paths = AppPaths::from_base("/c".into(), "/d".into(), "/k".into());
        assert_eq!(paths.config_file, PathBuf::from("/c/profiles.toml"));
        let moved = paths.with_config_file("/elsewhere/p.toml".into());
        assert_eq!(moved.config_file, PathBuf::from("/elsewhere/p.toml"));
    }
}
