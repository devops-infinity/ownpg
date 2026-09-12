use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    ChannelBinding, FILE_CAP_BYTES, Mode, PROFILE_FORMAT, SshTransport, SslMode, ToolGroup,
};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    pub pg_bindir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
