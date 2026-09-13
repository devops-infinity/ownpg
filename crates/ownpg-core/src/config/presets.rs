use std::path::{Path, PathBuf};

pub const FALLBACK_TCP_HOST: &str = "127.0.0.1";
pub const FALLBACK_TCP_USER: &str = "postgres";

#[must_use]
pub fn socket_directories() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/tmp"),
            PathBuf::from("/var/run/postgresql"),
            PathBuf::from("/run/postgresql"),
        ]
    } else if cfg!(windows) {
        Vec::new()
    } else {
        vec![
            PathBuf::from("/var/run/postgresql"),
            PathBuf::from("/run/postgresql"),
            PathBuf::from("/tmp"),
        ]
    }
}

#[must_use]
pub fn is_socket_directory(host: &str) -> bool {
    host.starts_with('/')
}

#[must_use]
pub fn socket_path(directory: &Path, port: u16) -> PathBuf {
    directory.join(format!(".s.PGSQL.{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_file_carries_the_port() {
        assert_eq!(
            socket_path(Path::new("/tmp"), 5432),
            PathBuf::from("/tmp/.s.PGSQL.5432")
        );
    }

    #[test]
    fn a_host_starting_with_a_slash_is_a_socket_directory() {
        assert!(is_socket_directory("/var/run/postgresql"));
        assert!(!is_socket_directory("localhost"));
        assert!(!is_socket_directory("127.0.0.1"));
    }

    #[cfg(unix)]
    #[test]
    fn every_unix_platform_probes_the_temp_directory_somewhere() {
        let dirs = socket_directories();
        assert!(dirs.contains(&PathBuf::from("/tmp")));
        assert!(dirs.contains(&PathBuf::from("/var/run/postgresql")));
    }
}
