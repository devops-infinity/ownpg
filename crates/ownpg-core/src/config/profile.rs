use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    AuditFailure, ChannelBinding, FILE_CAP_BYTES, KeychainScope, Mode, PROFILE_FORMAT, ResultText,
    SshTransport, SslMode, SslNegotiation, ToolGroup,
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
    pub keychain_scope: Option<KeychainScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_text: Option<ResultText>,
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
    pub sslnegotiation: Option<SslNegotiation>,
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
    pub audit_keep_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_on_failure: Option<AuditFailure>,
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
        let text = read_capped_handle(open_private(path)?, path)?;
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

pub const PROFILE_TEMPLATE: &str = r#"format = 1

[profiles.local]
mode = "read-only"
database = "your-database"
schema = "public"
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyTable {
    pub table: &'static str,
    pub keys: Vec<String>,
}

#[must_use]
pub fn key_reference() -> Vec<KeyTable> {
    let schema = serde_json::to_value(schemars::schema_for!(ProfileFile)).unwrap_or_default();
    let names = |pointer: &str| -> Vec<String> {
        schema
            .pointer(pointer)
            .and_then(serde_json::Value::as_object)
            .map(|properties| properties.keys().cloned().collect())
            .unwrap_or_default()
    };
    vec![
        KeyTable {
            table: "profiles.<name>",
            keys: names("/$defs/ProfileEntry/properties"),
        },
        KeyTable {
            table: "profiles.<name>.ssh",
            keys: names("/$defs/SshEntry/properties"),
        },
        KeyTable {
            table: "profiles.<name>.http",
            keys: names("/$defs/HttpEntry/properties"),
        },
    ]
}

pub fn open_file(path: &Path) -> Result<fs::File> {
    fs::File::open(path).map_err(|source| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source,
    })
}

pub fn read_capped(path: &Path) -> Result<String> {
    read_capped_handle(open_file(path)?, path)
}

pub fn read_capped_handle(file: fs::File, path: &Path) -> Result<String> {
    let bytes = read_capped_bytes(file, path)?;
    String::from_utf8(bytes).map_err(|error| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, error),
    })
}

pub fn read_capped_bytes(mut file: fs::File, path: &Path) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    Read::by_ref(&mut file)
        .take(FILE_CAP_BYTES + 1)
        .read_to_end(&mut buffer)
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
fn shared_mode(raw: u32) -> Option<u32> {
    let mode = raw & 0o777;
    (mode & 0o077 != 0).then_some(mode)
}

#[cfg(unix)]
pub fn open_permissions(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).map_err(|source| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(shared_mode(metadata.mode()))
}

#[cfg(not(unix))]
pub fn open_permissions(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(unix)]
pub fn handle_permissions(file: &fs::File, path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|source| Error::ConfigUnreadable {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(shared_mode(metadata.mode()))
}

#[cfg(not(unix))]
pub fn handle_permissions(_file: &fs::File, _path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

pub fn open_private(path: &Path) -> Result<fs::File> {
    let file = open_file(path)?;
    match handle_permissions(&file, path)? {
        Some(mode) => Err(Error::ConfigPermissions {
            path: path.to_path_buf(),
            mode,
        }),
        None => Ok(file),
    }
}

const TEMPORARY_PREFIX: &str = ".ownpg-";
const STALE_TEMPORARY_AGE: std::time::Duration = std::time::Duration::from_secs(86_400);

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    write_private_file(path, bytes, true)
}

pub fn create_private(path: &Path, bytes: &[u8]) -> Result<()> {
    write_private_file(path, bytes, false)
}

fn write_private_file(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let unwritable = |source: std::io::Error| Error::ConfigUnwritable {
        path: path.to_path_buf(),
        source,
    };
    let directory = path
        .parent()
        .ok_or_else(|| unwritable(std::io::Error::other("the path has no parent directory")))?;
    create_private_directory(directory).map_err(unwritable)?;
    sweep_stale_temporaries(directory);
    let mut temporary = tempfile::Builder::new()
        .prefix(TEMPORARY_PREFIX)
        .tempfile_in(directory)
        .map_err(unwritable)?;
    temporary.write_all(bytes).map_err(unwritable)?;
    temporary.flush().map_err(unwritable)?;
    temporary.as_file().sync_all().map_err(unwritable)?;
    restrict_to_owner(temporary.path()).map_err(unwritable)?;
    if replace {
        temporary
            .persist(path)
            .map_err(|error| unwritable(error.error))?;
    } else {
        temporary.persist_noclobber(path).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                unwritable(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "the file exists; pass --force to replace it",
                ))
            } else {
                unwritable(error.error)
            }
        })?;
    }
    #[cfg(unix)]
    {
        if let Ok(handle) = fs::File::open(directory)
            && let Err(error) = handle.sync_all()
        {
            tracing::debug!(%error, directory = %directory.display(), "the directory entry could not be flushed");
        }
    }
    Ok(())
}

fn sweep_stale_temporaries(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(TEMPORARY_PREFIX) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_TEMPORARY_AGE);
        if metadata.is_file()
            && stale
            && let Err(error) = fs::remove_file(entry.path())
        {
            tracing::warn!(%error, path = %entry.path().display(), "a stale temporary file could not be removed");
        }
    }
}

pub fn lock_profiles(path: &Path) -> Result<fs::File> {
    let mut lock_name = path.as_os_str().to_owned();
    lock_name.push(".lock");
    let lock_path = std::path::PathBuf::from(lock_name);
    let unwritable = |source: std::io::Error| Error::ConfigUnwritable {
        path: lock_path.clone(),
        source,
    };
    if let Some(directory) = lock_path.parent() {
        create_private_directory(directory).map_err(unwritable)?;
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&lock_path).map_err(unwritable)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err(unwritable(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "another ownpg config command is changing the profile file; try again when it finishes",
        ))),
        Err(fs::TryLockError::Error(source)) => Err(unwritable(source)),
    }
}

pub fn create_private_directory(directory: &Path) -> std::io::Result<()> {
    if directory.as_os_str().is_empty() || directory.is_dir() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
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
    fn the_starter_template_parses_and_carries_no_comment_lines() {
        let parsed: ProfileFile = toml::from_str(PROFILE_TEMPLATE).unwrap();
        assert_eq!(parsed, ProfileFile::example());
        assert!(
            PROFILE_TEMPLATE
                .lines()
                .all(|line| !line.trim_start().starts_with('#'))
        );
    }

    #[test]
    fn the_key_reference_lists_every_table_with_its_keys() {
        let reference = key_reference();
        let tables: Vec<&str> = reference.iter().map(|entry| entry.table).collect();
        assert_eq!(
            tables,
            [
                "profiles.<name>",
                "profiles.<name>.ssh",
                "profiles.<name>.http"
            ]
        );
        let profile = &reference[0].keys;
        for key in [
            "mode",
            "database",
            "schema",
            "audit_keep_files",
            "ssh",
            "http",
        ] {
            assert!(profile.iter().any(|known| known == key), "{key} missing");
        }
        assert!(reference[1].keys.iter().any(|key| key == "host"));
        assert!(reference[2].keys.iter().any(|key| key == "bind"));
        let schema = serde_json::to_value(schemars::schema_for!(ProfileFile)).unwrap();
        assert_eq!(
            reference[0].keys,
            property_names(&schema, "/$defs/ProfileEntry/properties")
        );
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

    #[cfg(unix)]
    #[test]
    fn a_private_open_refuses_every_shared_mode_and_accepts_an_owner_only_one() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = temp_file("secret", "owner only\n");
        let mut file = open_private(&path).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "owner only\n");
        for mode in [0o640, 0o604, 0o644, 0o660, 0o666] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let error = open_private(&path).unwrap_err();
            assert_eq!(error.id(), ErrorId::ConfigPermissions);
            assert!(error.to_string().contains(&format!("{mode:o}")), "{error}");
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(open_private(&path).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_private_open_reads_the_file_it_checked_even_when_the_path_is_swapped() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, path) = temp_file("secret", "owner only\n");
        let checked = open_private(&path).unwrap();
        let planted = dir.path().join("planted");
        fs::write(&planted, "someone else\n").unwrap();
        fs::set_permissions(&planted, fs::Permissions::from_mode(0o644)).unwrap();
        fs::rename(&planted, &path).unwrap();
        assert_eq!(read_capped_handle(checked, &path).unwrap(), "owner only\n");
        let error = open_private(&path).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigPermissions);
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
