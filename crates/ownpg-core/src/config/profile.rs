use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    ChannelBinding, FILE_CAP_BYTES, Mode, PROFILE_FORMAT, SshTransport, SslMode, ToolGroup,
};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostaddr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_keychain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sslmode: Option<SslMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sslrootcert: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sslcert: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sslkey: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_binding: Option<ChannelBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pooled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement_timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle_expiry_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_expiry_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_cap: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_cap: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_role: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_keep_files: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pg_bindir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<super::http::HttpEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SshEntry {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase_keychain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_new_host: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<SshTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_seconds: Option<u64>,
}

impl ProfileFile {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        refuse_open_permissions(path)?;
        let text = read_capped(path)?;
        let parsed: Self = toml::from_str(&text).map_err(|error| Error::ConfigMalformed {
            path: path.to_path_buf(),
            line: error.span().map(|span| line_of(&text, span.start)),
            detail: error.message().to_owned(),
        })?;
        let format = parsed.format.unwrap_or(PROFILE_FORMAT);
        if format > PROFILE_FORMAT {
            return Err(Error::ConfigFormat {
                path: path.to_path_buf(),
                found: format,
                supported: PROFILE_FORMAT,
            });
        }
        Ok(Some(parsed))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut on_disk = self.clone();
        on_disk.format = Some(PROFILE_FORMAT);
        let text = toml::to_string_pretty(&on_disk).map_err(|error| Error::ConfigUnwritable {
            path: path.to_path_buf(),
            source: std::io::Error::other(error.to_string()),
        })?;
        write_private(path, text.as_bytes())
    }

    pub fn profile(&self, name: &str, path: &Path) -> Result<&ProfileEntry> {
        self.profiles
            .get(name)
            .ok_or_else(|| Error::ProfileUnknown {
                name: name.to_owned(),
                path: path.to_path_buf(),
                known: self.profiles.keys().cloned().collect(),
            })
    }

    #[must_use]
    pub fn example() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "local".to_owned(),
            ProfileEntry {
                mode: Some(Mode::ReadOnly),
                database: Some("your-database".to_owned()),
                schema: Some("public".to_owned()),
                ..ProfileEntry::default()
            },
        );
        Self {
            format: Some(PROFILE_FORMAT),
            profiles,
        }
    }
}

pub const TEMPLATE: &str = r#"# OwnPG profile file. Every key is optional unless the tool says otherwise.
# Flags win over environment variables, which win over this file.
format = 1

[profiles.local]
# Mode: read-only, write-only, or read-write. Flag -m, env OWNPG_MODE. Default read-only.
mode = "read-only"
# The one database and the one schema this profile serves. Flag -d and -s, env OWNPG_DATABASE (PGDATABASE also applies) and OWNPG_SCHEMA.
database = "your-database"
schema = "public"
# Where the server is. Flag --host and --port, env OWNPG_HOST and OWNPG_PORT (PGHOST and PGPORT also apply).
# host = "127.0.0.1"
# port = 5432
# A numeric address that skips the DNS lookup for host. Env PGHOSTADDR.
# hostaddr = "127.0.0.1"
# A libpq connection string or a pg_service.conf service name, as an alternative to host, port, user, and database. Env OWNPG_DSN and PGSERVICE (PGSERVICEFILE and PGSYSCONFDIR locate the service file).
# dsn = "postgresql://user@host:5432/db"
# service = "mydb"
# The role to connect as. Flag -U, env OWNPG_USER (PGUSER also applies). Default: the operating system user.
# user = "app"
# How the password is found: a plain value here (the file must be private), the name of an environment variable, or the platform keychain (`ownpg config set-password local`). Env OWNPG_PASSWORD, PGPASSWORD, or a PGPASSFILE that only you can read.
# password = ""
# password_env = "APP_DB_PASSWORD"
# password_keychain = true
# TLS: disable, allow, prefer, require, verify-ca, or verify-full, plus the certificate files. Flag --sslmode and --sslrootcert, env OWNPG_SSLMODE, OWNPG_SSLROOTCERT, OWNPG_SSLCERT, OWNPG_SSLKEY (PGSSLMODE, PGSSLROOTCERT, PGSSLCERT, PGSSLKEY, PGSSLNEGOTIATION also apply). Default prefer.
# sslmode = "verify-full"
# sslrootcert = "/etc/ssl/certs/ca.pem"
# sslcert = "/home/me/.postgresql/postgresql.crt"
# sslkey = "/home/me/.postgresql/postgresql.key"
# SCRAM channel binding: disable, prefer, or require. Env PGCHANNELBINDING. Default prefer.
# channel_binding = "prefer"
# Seconds to wait for the connection. Env OWNPG_CONNECT_TIMEOUT or PGCONNECT_TIMEOUT. Default 10.
# connect_timeout_seconds = 10
# The application_name PostgreSQL shows in pg_stat_activity. Env PGAPPNAME.
# application_name = "ownpg"
# Extra libpq options passed as-is. Env PGOPTIONS.
# options = "-c work_mem=64MB"
# true when the target is a transaction pooler such as PgBouncer: settings go per transaction and every name must be schema-qualified. Env OWNPG_POOLED.
# pooled = false
# Server-side limits in seconds and result caps. Env OWNPG_STATEMENT_TIMEOUT, OWNPG_LOCK_TIMEOUT, OWNPG_TRANSACTION_TIMEOUT, OWNPG_HANDLE_EXPIRY, OWNPG_CURSOR_EXPIRY, OWNPG_ROW_CAP, OWNPG_BYTE_CAP.
# statement_timeout_seconds = 30
# lock_timeout_seconds = 5
# transaction_timeout_seconds = 300
# handle_expiry_seconds = 60
# cursor_expiry_seconds = 30
# row_cap = 100
# byte_cap = 262144
# Refuse to run as a superuser or a role with CREATEROLE, CREATEDB, or BYPASSRLS. Flag --strict-role, env OWNPG_STRICT_ROLE. Default on under --http.
# strict_role = true
# Tool groups to load besides the read-only set: write, transactions, ddl, roles, maintenance, monitoring, host. Flag --tools, env OWNPG_TOOLS.
# tools = ["write", "transactions"]
# Never prompt; fail instead. Flag --no-input, env OWNPG_NO_INPUT (CI also counts).
# no_input = false
# The audit log: on by default, one JSON line per call, rotated at audit_max_bytes, with audit_keep_files rotated files kept (0 keeps all). Flag --no-audit and --audit-path, env OWNPG_AUDIT, OWNPG_AUDIT_PATH, OWNPG_AUDIT_MAX_BYTES, OWNPG_AUDIT_KEEP_FILES.
# audit = true
# audit_path = "/var/log/ownpg/audit.jsonl"
# audit_max_bytes = 52428800
# audit_keep_files = 0
# Where pg_dump and friends live, and where their output files go. Flag --pg-bindir and --output-dir, env OWNPG_PG_BINDIR and OWNPG_OUTPUT_DIR. Both must be absolute.
# pg_bindir = "/usr/lib/postgresql/18/bin"
# output_dir = "/var/backups/ownpg"

# Reach the database through an SSH bastion. Flag --ssh, --ssh-transport, and --ssh-trust-new-host; env OWNPG_SSH, OWNPG_SSH_TRANSPORT, OWNPG_SSH_TRUST_NEW_HOST. The passphrase can come from the platform keychain (`ownpg config set-ssh-passphrase local`).
# [profiles.local.ssh]
# host = "bastion.example"
# port = 22
# user = "deploy"
# key_file = "/home/me/.ssh/id_ed25519"
# agent = true
# password_env = "BASTION_PASSPHRASE"
# passphrase_keychain = true
# trust_new_host = false
# transport = "in-process"
# jump = ["first-hop.example", "second-hop.example"]
# known_hosts = "/home/me/.ssh/known_hosts"
# config_file = "/home/me/.ssh/config"
# connect_timeout_seconds = 10

# Streamable HTTP settings, used with `ownpg serve --http`. Flag --bind and --auth, env OWNPG_BIND, OWNPG_PUBLIC_URL, OWNPG_ALLOWED_HOSTS, OWNPG_ALLOWED_ORIGINS, OWNPG_BODY_CAP_BYTES, OWNPG_RATE_LIMIT_PER_MINUTE, OWNPG_OLDER_CLIENT_SESSIONS, OWNPG_SHUTDOWN_SECONDS, OWNPG_POOL_SIZE, OWNPG_MAX_CONNECTIONS, OWNPG_TRUSTED_PROXIES, OWNPG_AUTH, OWNPG_BEARER_TOKENS_FILE, OWNPG_OAUTH_ISSUER, OWNPG_OAUTH_JWKS_URL, OWNPG_OAUTH_AUDIENCE, OWNPG_STATE_KEY_FILE, OTEL_EXPORTER_OTLP_ENDPOINT.
# [profiles.local.http]
# bind = "127.0.0.1:8765"
# public_url = "https://db.example/mcp"
# allowed_hosts = ["db.example"]
# allowed_origins = ["https://app.example"]
# body_cap_bytes = 1048576
# rate_limit_per_minute = 60
# older_client_sessions = false
# shutdown_seconds = 10
# pool_size = 4
# max_connections = 1024
# trusted_proxies = ["10.0.0.0/8"]
# auth = "bearer"
# tokens_file = "/etc/ownpg/tokens"
# oauth_issuer = "https://issuer.example"
# oauth_jwks_url = "https://issuer.example/.well-known/jwks.json"
# oauth_audience = "ownpg"
# state_key_file = "/etc/ownpg/state.key"
# otel_endpoint = "http://127.0.0.1:4318"
"#;

pub fn read_capped(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|source| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buffer = String::new();
    Read::by_ref(&mut file)
        .take(FILE_CAP_BYTES + 1)
        .read_to_string(&mut buffer)
        .map_err(|source| Error::ConfigUnreadable {
            path: path.to_path_buf(),
            source,
        })?;
    if buffer.len() as u64 > FILE_CAP_BYTES {
        return Err(Error::ConfigTooLarge {
            path: path.to_path_buf(),
            cap_bytes: FILE_CAP_BYTES,
        });
    }
    Ok(buffer)
}

#[cfg(unix)]
pub fn open_permissions(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).map_err(|source| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source,
    })?;
    let mode = metadata.mode() & 0o777;
    Ok((mode & 0o077 != 0).then_some(mode))
}

#[cfg(not(unix))]
pub fn open_permissions(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

pub fn refuse_open_permissions(path: &Path) -> Result<()> {
    match open_permissions(path)? {
        Some(mode) => Err(Error::ConfigPermissions {
            path: path.to_path_buf(),
            mode,
        }),
        None => Ok(()),
    }
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let unwritable = |source: std::io::Error| Error::ConfigUnwritable {
        path: path.to_path_buf(),
        source,
    };
    let directory = path
        .parent()
        .ok_or_else(|| unwritable(std::io::Error::other("the path has no parent directory")))?;
    create_private_directory(directory).map_err(unwritable)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".ownpg-")
        .tempfile_in(directory)
        .map_err(unwritable)?;
    temporary.write_all(bytes).map_err(unwritable)?;
    temporary.flush().map_err(unwritable)?;
    restrict_to_owner(temporary.path()).map_err(unwritable)?;
    temporary
        .persist(path)
        .map_err(|error| unwritable(error.error))?;
    Ok(())
}

pub fn create_private_directory(directory: &Path) -> std::io::Result<()> {
    if directory.as_os_str().is_empty() || directory.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(directory)?;
    restrict_directory_to_owner(directory)
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_directory_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_directory_to_owner(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset).map_or(1, |head| {
        head.bytes().filter(|byte| *byte == b'\n').count() + 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property_names(schema: &serde_json::Value, pointer: &str) -> Vec<String> {
        schema
            .pointer(pointer)
            .and_then(serde_json::Value::as_object)
            .map(|properties| properties.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_template_loads_and_names_every_key_with_its_uncommented_form_parsing_too() {
        let parsed: ProfileFile = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(parsed, ProfileFile::example());
        let schema = serde_json::to_value(schemars::schema_for!(ProfileFile)).unwrap();
        for pointer in [
            "/$defs/ProfileEntry/properties",
            "/$defs/SshEntry/properties",
            "/$defs/HttpEntry/properties",
        ] {
            for key in property_names(&schema, pointer) {
                let commented = format!("# {key} = ");
                let plain = format!("{key} = ");
                let table = format!("# [profiles.local.{key}]");
                assert!(
                    TEMPLATE.contains(&commented)
                        || TEMPLATE.contains(&plain)
                        || TEMPLATE.contains(&table),
                    "the profile template does not show `{key}`"
                );
            }
        }
        let uncommented: String = TEMPLATE
            .lines()
            .map(|line| {
                line.strip_prefix("# ")
                    .filter(|rest| rest.contains(" = ") || rest.starts_with('['))
                    .unwrap_or(line)
            })
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let full: ProfileFile = toml::from_str(&uncommented).unwrap();
        let entry = full.profiles.get("local").unwrap();
        assert!(entry.ssh.is_some() && entry.http.is_some());
    }

    #[test]
    fn the_template_names_every_environment_variable_the_resolver_reads() {
        let sources = [
            include_str!("resolve.rs"),
            include_str!("http.rs"),
            include_str!("libpq.rs"),
        ];
        let mut read = std::collections::BTreeSet::new();
        for source in sources {
            for (index, _) in source.match_indices("\"OWNPG_") {
                let rest = &source[index + 1..];
                let end = rest.find('"').unwrap_or(rest.len());
                read.insert(rest[..end].to_owned());
            }
            for (index, _) in source.match_indices("(\"PG") {
                let rest = &source[index + 2..];
                let end = rest.find('"').unwrap_or(rest.len());
                read.insert(rest[..end].to_owned());
            }
        }
        let template_only = ["OWNPG_PROFILE", "OWNPG_BEARER_TOKENS"];
        for variable in &read {
            if template_only.contains(&variable.as_str()) {
                continue;
            }
            assert!(
                TEMPLATE.contains(variable.as_str()),
                "the profile template does not mention {variable}"
            );
        }
        assert!(read.len() > 40, "{read:?}");
    }

    #[test]
    fn the_profile_keys_are_pinned_to_the_profile_format() {
        let schema = serde_json::to_value(schemars::schema_for!(ProfileFile)).unwrap();
        let mut keys = Vec::new();
        keys.push(format!("format {PROFILE_FORMAT}"));
        keys.push(format!(
            "profile: {}",
            property_names(&schema, "/$defs/ProfileEntry/properties").join(", ")
        ));
        keys.push(format!(
            "ssh: {}",
            property_names(&schema, "/$defs/SshEntry/properties").join(", ")
        ));
        keys.push(format!(
            "http: {}",
            property_names(&schema, "/$defs/HttpEntry/properties").join(", ")
        ));
        insta::assert_snapshot!(
            format!("profile-keys-format-{PROFILE_FORMAT}"),
            keys.join("\n")
        );
    }
    use crate::error::ErrorId;

    fn temp_file(name: &str, text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::write(&path, text).unwrap();
        restrict_to_owner(&path).unwrap();
        (dir, path)
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ProfileFile::load(&dir.path().join("none.toml")).unwrap(),
            None
        );
    }

    #[test]
    fn a_profile_round_trips_through_the_file() {
        let example = ProfileFile::example();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("profiles.toml");
        example.save(&path).unwrap();
        let loaded = ProfileFile::load(&path).unwrap().unwrap();
        assert_eq!(loaded, example);
        assert_eq!(
            loaded.profile("local", &path).unwrap().mode,
            Some(Mode::ReadOnly)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_saved_file_and_its_directory_belong_to_the_owner_only() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private").join("profiles.toml");
        ProfileFile::example().save(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn an_unknown_key_names_the_file_the_line_and_the_key() {
        let (_dir, path) = temp_file(
            "profiles.toml",
            "format = 1\n\n[profiles.local]\ndatabase = \"app\"\nstrict_rol = true\n",
        );
        let error = ProfileFile::load(&path).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigMalformed);
        let text = error.to_string();
        assert!(text.contains("at line 5"), "{text}");
        assert!(text.contains("strict_rol"), "{text}");
    }

    #[test]
    fn a_newer_format_is_refused_with_an_upgrade_message() {
        let (_dir, path) = temp_file("profiles.toml", "format = 2\n");
        let error = ProfileFile::load(&path).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigFormat);
        assert!(error.remedy().contains("Upgrade"));
    }

    #[test]
    fn a_format_one_file_written_by_the_first_release_still_loads() {
        let text = r#"format = 1

[profiles.local]
database = "app"
schema = "public"
mode = "read-only"
host = "127.0.0.1"
port = 5432
user = "app"
tools = ["monitoring"]

[profiles.local.http]
bind = "127.0.0.1:8765"
auth = "bearer"
tokens_file = "/etc/ownpg/tokens"
"#;
        let (_dir, path) = temp_file("profiles.toml", text);
        let loaded = ProfileFile::load(&path).unwrap().unwrap();
        assert_eq!(loaded.format, Some(PROFILE_FORMAT));
        let local = loaded.profile("local", &path).unwrap();
        assert_eq!(local.database.as_deref(), Some("app"));
        assert_eq!(local.mode, Some(Mode::ReadOnly));
        assert_eq!(
            local.http.as_ref().and_then(|http| http.auth),
            Some(super::super::http::AuthMode::Bearer)
        );
    }

    #[test]
    fn a_missing_format_reads_as_the_current_one() {
        let (_dir, path) = temp_file("profiles.toml", "[profiles.a]\ndatabase = \"x\"\n");
        let loaded = ProfileFile::load(&path).unwrap().unwrap();
        assert_eq!(loaded.format, None);
        assert!(loaded.profiles.contains_key("a"));
    }

    #[test]
    fn an_unknown_profile_lists_the_known_ones() {
        let file = ProfileFile::example();
        let error = file.profile("staging", Path::new("/p")).unwrap_err();
        assert_eq!(error.id(), ErrorId::ProfileUnknown);
        assert_eq!(error.remedy(), "Use one of: local.");
    }

    #[cfg(unix)]
    #[test]
    fn a_readable_by_others_file_is_refused_and_never_tightened() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let (_dir, path) = temp_file("profiles.toml", "format = 1\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let error = ProfileFile::load(&path).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigPermissions);
        assert!(error.to_string().contains("644"), "{error}");
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
    }

    #[test]
    fn a_file_over_the_cap_is_refused() {
        let big = "x".repeat((FILE_CAP_BYTES + 1) as usize);
        let (_dir, path) = temp_file("profiles.toml", &big);
        let error = ProfileFile::load(&path).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigTooLarge);
    }

    #[test]
    fn the_ssh_section_needs_a_host() {
        let (_dir, path) = temp_file(
            "profiles.toml",
            "[profiles.a]\ndatabase = \"x\"\n[profiles.a.ssh]\nuser = \"deploy\"\n",
        );
        let error = ProfileFile::load(&path).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigMalformed);
        assert!(error.to_string().contains("host"), "{error}");
    }
}
