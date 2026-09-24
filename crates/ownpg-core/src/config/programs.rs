use std::path::{Path, PathBuf};

use super::Settings;

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
