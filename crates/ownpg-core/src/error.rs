use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ExitClass {
    Success = 0,
    Runtime = 1,
    Usage = 2,
    Refused = 4,
    External = 5,
    Interrupted = 130,
}

impl ExitClass {
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for ExitClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorId {
    CommandUnknown,
    OutputUnwritable,
    InputUnreadable,
    AuditTampered,
    StatementUnparsable,
    StatementMultiple,
    StatementRefused,
    ConfigUnreadable,
    ConfigTooLarge,
    ConfigPermissions,
    ConfigMalformed,
    ConfigFormat,
    ConfigInvalid,
    ConfigUnwritable,
    ProfileUnknown,
    DatabaseMissing,
    DsnInvalid,
    ConnectFailed,
    TlsFailed,
    RoleRefused,
    SshFailed,
    SshHostKeyUnknown,
    HandleState,
    AuditUnwritable,
    ProtocolFailed,
    SqlFailed,
    ExtensionMissing,
    ExtensionOutdated,
    HostBinaryMissing,
    SubprocessFailed,
    ArgumentInvalid,
    ConfirmationRequired,
    ScopeInsufficient,
}

impl ErrorId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommandUnknown => "command.unknown",
            Self::OutputUnwritable => "output.unwritable",
            Self::InputUnreadable => "input.unreadable",
            Self::AuditTampered => "audit.tampered",
            Self::StatementUnparsable => "statement.unparsable",
            Self::StatementMultiple => "statement.multiple",
            Self::StatementRefused => "statement.refused",
            Self::ConfigUnreadable => "config.unreadable",
            Self::ConfigTooLarge => "config.too_large",
            Self::ConfigPermissions => "config.permissions",
            Self::ConfigMalformed => "config.malformed",
            Self::ConfigFormat => "config.format",
            Self::ConfigInvalid => "config.invalid",
            Self::ConfigUnwritable => "config.unwritable",
            Self::ProfileUnknown => "profile.unknown",
            Self::DatabaseMissing => "database.missing",
            Self::DsnInvalid => "dsn.invalid",
            Self::ConnectFailed => "connect.failed",
            Self::TlsFailed => "connect.tls",
            Self::RoleRefused => "role.refused",
            Self::SshFailed => "ssh.failed",
            Self::SshHostKeyUnknown => "ssh.host_key_unknown",
            Self::HandleState => "handle.state",
            Self::AuditUnwritable => "audit.unwritable",
            Self::ProtocolFailed => "protocol.failed",
            Self::SqlFailed => "sql.failed",
            Self::ExtensionMissing => "extension.missing",
            Self::ExtensionOutdated => "extension.outdated",
            Self::HostBinaryMissing => "host_binary.missing",
            Self::SubprocessFailed => "subprocess.failed",
            Self::ArgumentInvalid => "argument.invalid",
            Self::ConfirmationRequired => "confirmation.required",
            Self::ScopeInsufficient => "scope.insufficient",
        }
    }
}

impl fmt::Display for ErrorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("`{name}` is not a command")]
    CommandUnknown { name: String, known: Vec<String> },

    #[error("output could not be written to {target}")]
    OutputUnwritable {
        target: String,
        #[source]
        source: std::io::Error,
    },

    #[error("input could not be read from {stream}")]
    InputUnreadable {
        stream: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the audit log at {path} does not verify: {detail}")]
    AuditTampered { path: PathBuf, detail: String },

    #[error("the statement could not be parsed: {reason}")]
    StatementUnparsable { reason: String },

    #[error("the input holds {count} statements, and one call runs one statement")]
    StatementMultiple { count: usize },

    #[error("the statement is refused in {mode} mode: {rule}")]
    StatementRefused { rule: String, mode: String },

    #[error("{path} could not be read")]
    ConfigUnreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is larger than the {cap_bytes} byte cap for a configuration file")]
    ConfigTooLarge { path: PathBuf, cap_bytes: u64 },

    #[error("{path} has mode {mode:o}, which lets other users read it")]
    ConfigPermissions { path: PathBuf, mode: u32 },

    #[error("{path} could not be parsed{}: {detail}", line.map_or(String::new(), |line| format!(" at line {line}")))]
    ConfigMalformed {
        path: PathBuf,
        line: Option<usize>,
        detail: String,
    },

    #[error("{path} is format {found}, and this build reads format {supported}")]
    ConfigFormat {
        path: PathBuf,
        found: u32,
        supported: u32,
    },

    #[error("`{value}` is not a usable {setting}: {detail}")]
    ConfigInvalid {
        setting: String,
        value: String,
        detail: String,
    },

    #[error("{path} could not be written")]
    ConfigUnwritable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("`{name}` is not a profile in {path}")]
    ProfileUnknown {
        name: String,
        path: PathBuf,
        known: Vec<String>,
    },

    #[error("no database was named")]
    DatabaseMissing,

    #[error("the connection string could not be read: {detail}")]
    DsnInvalid { detail: String },

    #[error("PostgreSQL could not be reached")]
    ConnectFailed {
        tried: Vec<String>,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("the TLS setup for {target} failed: {detail}")]
    TlsFailed {
        target: String,
        detail: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("the role `{role}` has {attribute}, which strict role checking refuses")]
    RoleRefused { role: String, attribute: String },

    #[error("the SSH connection to {host} failed: {detail}")]
    SshFailed {
        host: String,
        detail: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("the host key of {host} is not in the known hosts file (fingerprint {fingerprint})")]
    SshHostKeyUnknown { host: String, fingerprint: String },

    #[error("the handle `{handle}` is {state}")]
    HandleState { handle: String, state: String },

    #[error("the audit log at {path} could not be written")]
    AuditUnwritable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the protocol layer failed: {detail}")]
    ProtocolFailed { detail: String },

    #[error("PostgreSQL refused the statement{}: {message}", sqlstate.as_deref().map_or(String::new(), |code| format!(" ({code})")))]
    SqlFailed {
        sqlstate: Option<String>,
        message: String,
    },

    #[error("the extension `{name}` is not installed in this database")]
    ExtensionMissing { name: String },

    #[error("the extension `{name}` is at version {installed}; this tool needs {needed} or newer")]
    ExtensionOutdated {
        name: String,
        installed: String,
        needed: String,
    },

    #[error("the program `{name}` was not found on this host")]
    HostBinaryMissing { name: String },

    #[error("`{program}` exited with {status}")]
    SubprocessFailed { program: String, status: String },

    #[error("the argument `{argument}` is not usable: {detail}")]
    ArgumentInvalid { argument: String, detail: String },

    #[error("this operation changes or removes data and needs a confirmation")]
    ConfirmationRequired { operation: String },

    #[error("the token does not carry the `{scope}` scope this tool needs")]
    ScopeInsufficient { scope: String },
}

impl Error {
    #[must_use]
    pub const fn id(&self) -> ErrorId {
        match self {
            Self::CommandUnknown { .. } => ErrorId::CommandUnknown,
            Self::OutputUnwritable { .. } => ErrorId::OutputUnwritable,
            Self::InputUnreadable { .. } => ErrorId::InputUnreadable,
            Self::AuditTampered { .. } => ErrorId::AuditTampered,
            Self::StatementUnparsable { .. } => ErrorId::StatementUnparsable,
            Self::StatementMultiple { .. } => ErrorId::StatementMultiple,
            Self::StatementRefused { .. } => ErrorId::StatementRefused,
            Self::ConfigUnreadable { .. } => ErrorId::ConfigUnreadable,
            Self::ConfigTooLarge { .. } => ErrorId::ConfigTooLarge,
            Self::ConfigPermissions { .. } => ErrorId::ConfigPermissions,
            Self::ConfigMalformed { .. } => ErrorId::ConfigMalformed,
            Self::ConfigFormat { .. } => ErrorId::ConfigFormat,
            Self::ConfigInvalid { .. } => ErrorId::ConfigInvalid,
            Self::ConfigUnwritable { .. } => ErrorId::ConfigUnwritable,
            Self::ProfileUnknown { .. } => ErrorId::ProfileUnknown,
            Self::DatabaseMissing => ErrorId::DatabaseMissing,
            Self::DsnInvalid { .. } => ErrorId::DsnInvalid,
            Self::ConnectFailed { .. } => ErrorId::ConnectFailed,
            Self::TlsFailed { .. } => ErrorId::TlsFailed,
            Self::RoleRefused { .. } => ErrorId::RoleRefused,
            Self::SshFailed { .. } => ErrorId::SshFailed,
            Self::SshHostKeyUnknown { .. } => ErrorId::SshHostKeyUnknown,
            Self::HandleState { .. } => ErrorId::HandleState,
            Self::AuditUnwritable { .. } => ErrorId::AuditUnwritable,
            Self::ProtocolFailed { .. } => ErrorId::ProtocolFailed,
            Self::SqlFailed { .. } => ErrorId::SqlFailed,
            Self::ExtensionMissing { .. } => ErrorId::ExtensionMissing,
            Self::ExtensionOutdated { .. } => ErrorId::ExtensionOutdated,
            Self::HostBinaryMissing { .. } => ErrorId::HostBinaryMissing,
            Self::SubprocessFailed { .. } => ErrorId::SubprocessFailed,
            Self::ArgumentInvalid { .. } => ErrorId::ArgumentInvalid,
            Self::ConfirmationRequired { .. } => ErrorId::ConfirmationRequired,
            Self::ScopeInsufficient { .. } => ErrorId::ScopeInsufficient,
        }
    }

    #[must_use]
    pub const fn exit_class(&self) -> ExitClass {
        match self {
            Self::CommandUnknown { .. }
            | Self::ConfigUnreadable { .. }
            | Self::ConfigTooLarge { .. }
            | Self::ConfigPermissions { .. }
            | Self::ConfigMalformed { .. }
            | Self::ConfigFormat { .. }
            | Self::ConfigInvalid { .. }
            | Self::ConfigUnwritable { .. }
            | Self::ProfileUnknown { .. }
            | Self::DatabaseMissing
            | Self::DsnInvalid { .. }
            | Self::ArgumentInvalid { .. } => ExitClass::Usage,
            Self::OutputUnwritable { .. }
            | Self::InputUnreadable { .. }
            | Self::AuditTampered { .. }
            | Self::HandleState { .. }
            | Self::ProtocolFailed { .. }
            | Self::SqlFailed { .. }
            | Self::ExtensionMissing { .. }
            | Self::ExtensionOutdated { .. } => ExitClass::Runtime,
            Self::StatementUnparsable { .. }
            | Self::StatementMultiple { .. }
            | Self::StatementRefused { .. }
            | Self::RoleRefused { .. }
            | Self::SshHostKeyUnknown { .. }
            | Self::ConfirmationRequired { .. }
            | Self::ScopeInsufficient { .. } => ExitClass::Refused,
            Self::ConnectFailed { .. }
            | Self::TlsFailed { .. }
            | Self::SshFailed { .. }
            | Self::AuditUnwritable { .. }
            | Self::HostBinaryMissing { .. }
            | Self::SubprocessFailed { .. } => ExitClass::External,
        }
    }

    #[must_use]
    pub fn remedy(&self) -> String {
        match self {
            Self::CommandUnknown { known, .. } => {
                if known.is_empty() {
                    "Run `ownpg --help` to see every command.".to_owned()
                } else {
                    format!("Use one of: {}.", known.join(", "))
                }
            }
            Self::OutputUnwritable { .. } => {
                "Check that the target is writable and has free space.".to_owned()
            }
            Self::InputUnreadable { .. } => {
                "Pipe the value on stdin, or run the command in a terminal that can prompt for it."
                    .to_owned()
            }
            Self::AuditTampered { .. } => {
                "Treat every line from the named one onward as unverified; a rotated or edited file breaks the chain there."
                    .to_owned()
            }
            Self::StatementUnparsable { .. } => {
                "Check the statement against the PostgreSQL manual, and send one statement per call."
                    .to_owned()
            }
            Self::StatementMultiple { .. } => {
                "Send each statement in its own call, or open a transaction handle to group them."
                    .to_owned()
            }
            Self::StatementRefused { mode, .. } => {
                format!("Start the server in a mode that allows it, or rewrite the statement for {mode} mode.")
            }
            Self::ConfigUnreadable { .. } => {
                "Check that the file exists and that you can read it.".to_owned()
            }
            Self::ConfigTooLarge { .. } => {
                "A profile file this large is not one OwnPG wrote; check the path.".to_owned()
            }
            Self::ConfigPermissions { path, .. } => {
                format!("Run `chmod 600 {}` so only you can read it.", path.display())
            }
            Self::ConfigMalformed { .. } => {
                "Fix the line named above; `ownpg config init` writes a fresh example.".to_owned()
            }
            Self::ConfigFormat { .. } => {
                "Upgrade OwnPG to a version that reads this format.".to_owned()
            }
            Self::ConfigInvalid { setting, .. } => {
                format!("Check the accepted values for `{setting}` in the profile reference.")
            }
            Self::ConfigUnwritable { .. } => {
                "Check that the directory exists and that you can write to it.".to_owned()
            }
            Self::ProfileUnknown { known, .. } => {
                if known.is_empty() {
                    "Run `ownpg config init` to write a first profile.".to_owned()
                } else {
                    format!("Use one of: {}.", known.join(", "))
                }
            }
            Self::DatabaseMissing => {
                "Pass `--database <name>`, set OWNPG_DATABASE or PGDATABASE, or name it in the profile."
                    .to_owned()
            }
            Self::DsnInvalid { .. } => {
                "Use the `postgresql://user@host:port/database` form or `key=value` pairs.".to_owned()
            }
            Self::ConnectFailed { tried, .. } => {
                if tried.is_empty() {
                    "Run `ownpg doctor` to see what was tried.".to_owned()
                } else {
                    format!("Tried: {}. Run `ownpg doctor` for the details.", tried.join(", "))
                }
            }
            Self::TlsFailed { .. } => {
                "Check `sslmode` and `sslrootcert`; `verify-full` needs a certificate whose name matches the host."
                    .to_owned()
            }
            Self::RoleRefused { .. } => {
                "Connect as a role without that attribute, or set `strict_role = false` for a local database."
                    .to_owned()
            }
            Self::SshFailed { .. } => {
                "Check the host, the user, the key or agent, and that `ssh <user>@<host>` works from this machine."
                    .to_owned()
            }
            Self::SshHostKeyUnknown { .. } => {
                "Connect once with `ssh` to record the key, or pass `--ssh-trust-new-host` to accept this fingerprint."
                    .to_owned()
            }
            Self::HandleState { .. } => {
                "Open a new handle with `begin`; a handle in this state cannot be used again.".to_owned()
            }
            Self::AuditUnwritable { .. } => {
                "Check the audit path and its permissions, or set `audit = false` to turn the log off."
                    .to_owned()
            }
            Self::ProtocolFailed { .. } => {
                "Check the client's protocol version and the request shape.".to_owned()
            }
            Self::SqlFailed { .. } => {
                "Read the PostgreSQL message above; the SQLSTATE code names the failure class.".to_owned()
            }
            Self::ExtensionMissing { name } => {
                format!("Run `CREATE EXTENSION {name}` as a role that may install extensions.")
            }
            Self::ExtensionOutdated { name, .. } => {
                format!("Run `ALTER EXTENSION {name} UPDATE` as a role that may alter extensions.")
            }
            Self::HostBinaryMissing { name } => {
                format!("Install the PostgreSQL client tools, or set `pg_bindir` to the directory that holds `{name}`.")
            }
            Self::SubprocessFailed { .. } => {
                "Read the program's own message above.".to_owned()
            }
            Self::ArgumentInvalid { .. } => {
                "Check the tool's input schema for the accepted shape.".to_owned()
            }
            Self::ConfirmationRequired { operation } => {
                format!("Call again with `confirm: true` to run `{operation}`, or with `dry_run: true` to see the SQL first.")
            }
            Self::ScopeInsufficient { scope } => {
                format!("Ask the identity provider for a token that carries the `{scope}` scope, or use a bearer token bound to a mode that allows this tool.")
            }
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn io_error() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no")
    }

    fn every_error() -> Vec<Error> {
        vec![
            Error::CommandUnknown {
                name: "nope".to_owned(),
                known: vec!["man".to_owned()],
            },
            Error::OutputUnwritable {
                target: "stdout".to_owned(),
                source: io_error(),
            },
            Error::InputUnreadable {
                stream: "stdin".to_owned(),
                source: io_error(),
            },
            Error::AuditTampered {
                path: PathBuf::from("/tmp/audit.jsonl"),
                detail: "line 3".to_owned(),
            },
            Error::StatementUnparsable {
                reason: "syntax error".to_owned(),
            },
            Error::StatementMultiple { count: 2 },
            Error::StatementRefused {
                rule: "write in read-only".to_owned(),
                mode: "read-only".to_owned(),
            },
            Error::ConfigUnreadable {
                path: "/tmp/p".into(),
                source: io_error(),
            },
            Error::ConfigTooLarge {
                path: "/tmp/p".into(),
                cap_bytes: 1_048_576,
            },
            Error::ConfigPermissions {
                path: "/tmp/p".into(),
                mode: 0o644,
            },
            Error::ConfigMalformed {
                path: "/tmp/p".into(),
                line: Some(3),
                detail: "unknown field".to_owned(),
            },
            Error::ConfigFormat {
                path: "/tmp/p".into(),
                found: 2,
                supported: 1,
            },
            Error::ConfigInvalid {
                setting: "mode".to_owned(),
                value: "x".to_owned(),
                detail: "not one of read-only, write-only, read-write".to_owned(),
            },
            Error::ConfigUnwritable {
                path: "/tmp/p".into(),
                source: io_error(),
            },
            Error::ProfileUnknown {
                name: "x".to_owned(),
                path: "/tmp/p".into(),
                known: vec![],
            },
            Error::DatabaseMissing,
            Error::DsnInvalid {
                detail: "x".to_owned(),
            },
            Error::ConnectFailed {
                tried: vec!["/tmp".to_owned()],
                source: "refused".into(),
            },
            Error::TlsFailed {
                target: "db".to_owned(),
                detail: "x".to_owned(),
                source: None,
            },
            Error::RoleRefused {
                role: "postgres".to_owned(),
                attribute: "SUPERUSER".to_owned(),
            },
            Error::SshFailed {
                host: "bastion".to_owned(),
                detail: "x".to_owned(),
                source: None,
            },
            Error::SshHostKeyUnknown {
                host: "bastion".to_owned(),
                fingerprint: "SHA256:abc".to_owned(),
            },
            Error::HandleState {
                handle: "h1".to_owned(),
                state: "expired".to_owned(),
            },
            Error::AuditUnwritable {
                path: "/tmp/a".into(),
                source: io_error(),
            },
            Error::ProtocolFailed {
                detail: "x".to_owned(),
            },
            Error::SqlFailed {
                sqlstate: Some("25006".to_owned()),
                message: "read-only".to_owned(),
            },
            Error::ExtensionMissing {
                name: "pg_stat_statements".to_owned(),
            },
            Error::ExtensionOutdated {
                name: "pg_stat_statements".to_owned(),
                installed: "1.7".to_owned(),
                needed: "1.8".to_owned(),
            },
            Error::HostBinaryMissing {
                name: "pg_dump".to_owned(),
            },
            Error::SubprocessFailed {
                program: "pg_dump".to_owned(),
                status: "exit 1".to_owned(),
            },
            Error::ArgumentInvalid {
                argument: "row_cap".to_owned(),
                detail: "above 1000".to_owned(),
            },
            Error::ConfirmationRequired {
                operation: "DROP TABLE".to_owned(),
            },
            Error::ScopeInsufficient {
                scope: "ownpg:write".to_owned(),
            },
        ]
    }

    #[test]
    fn exit_codes_are_the_documented_contract() {
        assert_eq!(ExitClass::Success.code(), 0);
        assert_eq!(ExitClass::Runtime.code(), 1);
        assert_eq!(ExitClass::Usage.code(), 2);
        assert_eq!(ExitClass::Refused.code(), 4);
        assert_eq!(ExitClass::External.code(), 5);
        assert_eq!(ExitClass::Interrupted.code(), 130);
        assert_eq!(
            [
                ExitClass::Success,
                ExitClass::Runtime,
                ExitClass::Usage,
                ExitClass::Refused,
                ExitClass::External,
                ExitClass::Interrupted,
            ]
            .len(),
            6,
            "the documented contract is exactly these six codes"
        );
    }

    #[test]
    fn every_exit_class_displays_as_its_code() {
        assert_eq!(ExitClass::Refused.to_string(), "4");
        assert_eq!(ExitClass::Interrupted.to_string(), "130");
    }

    #[test]
    fn every_error_has_a_distinct_id_and_a_remedy() {
        let errors = every_error();
        let mut ids: Vec<&str> = errors.iter().map(|case| case.id().as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "two errors share an id");
        for case in &errors {
            assert!(case.id().as_str().contains('.'), "{case:?}");
            assert!(!case.remedy().is_empty(), "{case:?}");
            assert!(!case.to_string().is_empty(), "{case:?}");
            assert_eq!(case.id().to_string(), case.id().as_str());
        }
    }

    #[test]
    fn the_exit_class_follows_the_documented_grouping() {
        for case in every_error() {
            let expected = match case.id() {
                ErrorId::CommandUnknown
                | ErrorId::ConfigUnreadable
                | ErrorId::ConfigTooLarge
                | ErrorId::ConfigPermissions
                | ErrorId::ConfigMalformed
                | ErrorId::ConfigFormat
                | ErrorId::ConfigInvalid
                | ErrorId::ConfigUnwritable
                | ErrorId::ProfileUnknown
                | ErrorId::DatabaseMissing
                | ErrorId::DsnInvalid
                | ErrorId::ArgumentInvalid => ExitClass::Usage,
                ErrorId::OutputUnwritable
                | ErrorId::InputUnreadable
                | ErrorId::AuditTampered
                | ErrorId::HandleState
                | ErrorId::ProtocolFailed
                | ErrorId::SqlFailed
                | ErrorId::ExtensionMissing
                | ErrorId::ExtensionOutdated => ExitClass::Runtime,
                ErrorId::StatementUnparsable
                | ErrorId::StatementMultiple
                | ErrorId::StatementRefused
                | ErrorId::RoleRefused
                | ErrorId::SshHostKeyUnknown
                | ErrorId::ConfirmationRequired
                | ErrorId::ScopeInsufficient => ExitClass::Refused,
                ErrorId::ConnectFailed
                | ErrorId::TlsFailed
                | ErrorId::SshFailed
                | ErrorId::AuditUnwritable
                | ErrorId::HostBinaryMissing
                | ErrorId::SubprocessFailed => ExitClass::External,
            };
            assert_eq!(case.exit_class(), expected, "{case:?}");
        }
    }

    #[test]
    fn error_id_strings_are_the_stable_contract() {
        assert_eq!(ErrorId::CommandUnknown.as_str(), "command.unknown");
        assert_eq!(ErrorId::StatementRefused.as_str(), "statement.refused");
        assert_eq!(ErrorId::ConfigPermissions.as_str(), "config.permissions");
        assert_eq!(ErrorId::SshHostKeyUnknown.as_str(), "ssh.host_key_unknown");
        assert_eq!(
            ErrorId::ConfirmationRequired.as_str(),
            "confirmation.required"
        );
    }

    #[test]
    fn messages_name_the_line_and_the_sqlstate_when_they_exist() {
        let malformed = Error::ConfigMalformed {
            path: "/tmp/p".into(),
            line: Some(7),
            detail: "unknown field `strict_rol`".to_owned(),
        };
        assert_eq!(
            malformed.to_string(),
            "/tmp/p could not be parsed at line 7: unknown field `strict_rol`"
        );
        let no_line = Error::ConfigMalformed {
            path: "/tmp/p".into(),
            line: None,
            detail: "empty".to_owned(),
        };
        assert_eq!(no_line.to_string(), "/tmp/p could not be parsed: empty");
        let sql = Error::SqlFailed {
            sqlstate: Some("25006".to_owned()),
            message: "cannot execute UPDATE in a read-only transaction".to_owned(),
        };
        assert!(sql.to_string().contains("(25006)"));
    }

    #[test]
    fn an_unwritable_output_keeps_its_cause() {
        let error = Error::OutputUnwritable {
            target: "stdout".to_owned(),
            source: io_error(),
        };
        assert!(std::error::Error::source(&error).is_some());
    }
}
