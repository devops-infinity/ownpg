use std::path::Path;

use super::{KeychainScope, Settings, SshSettings};

pub const SERVICE: &str = "ownpg";

const BINDING_VERSION: &str = "ownpg-binding-v1";
const DIGEST_HEX_CHARS: usize = 32;
const READABLE_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target<'a> {
    pub user: &'a str,
    pub host: Option<&'a str>,
    pub port: u16,
}

#[must_use]
pub fn password_account(
    scope: KeychainScope,
    profile_file: &Path,
    target: Target<'_>,
    ssh: Option<&SshSettings>,
) -> String {
    let endpoint = format!("{}@{}:{}", target.user, host_key(target.host), target.port);
    let mut record = vec![
        BINDING_VERSION.to_owned(),
        "kind=pg-password".to_owned(),
        scope_line(scope, profile_file),
        format!("target=postgresql://{endpoint}"),
    ];
    if let Some(ssh) = ssh {
        record.extend(route_lines(ssh));
    }
    account("pg", &endpoint, &record)
}

#[must_use]
pub fn ssh_secret_account(scope: KeychainScope, profile_file: &Path, ssh: &SshSettings) -> String {
    let endpoint = bastion(ssh);
    let mut record = vec![
        BINDING_VERSION.to_owned(),
        "kind=ssh-secret".to_owned(),
        scope_line(scope, profile_file),
        format!("target=ssh://{endpoint}"),
    ];
    record.extend(
        ssh.jump
            .value
            .iter()
            .map(|jump| format!("via=ssh://{}", jump.trim())),
    );
    account("ssh", &endpoint, &record)
}

#[must_use]
pub fn password_account_for(settings: &Settings) -> String {
    let connection = &settings.connection;
    password_account(
        settings.keychain_scope.value,
        &settings.paths.config_file,
        Target {
            user: &connection.user.value,
            host: connection.host.as_ref().map(|host| host.value.as_str()),
            port: connection.port.value,
        },
        settings.ssh.as_ref(),
    )
}

#[must_use]
pub fn ssh_secret_account_for(settings: &Settings) -> Option<String> {
    settings.ssh.as_ref().map(|ssh| {
        ssh_secret_account(
            settings.keychain_scope.value,
            &settings.paths.config_file,
            ssh,
        )
    })
}

fn route_lines(ssh: &SshSettings) -> Vec<String> {
    ssh.jump
        .value
        .iter()
        .map(|jump| format!("via=ssh://{}", jump.trim()))
        .chain(std::iter::once(format!("via=ssh://{}", bastion(ssh))))
        .collect()
}

fn bastion(ssh: &SshSettings) -> String {
    format!(
        "{}@{}:{}",
        ssh.user.as_ref().map_or("", |user| user.value.as_str()),
        host_key(Some(&ssh.host.value)),
        ssh.port.value
    )
}

fn host_key(host: Option<&str>) -> String {
    match host.map(str::trim) {
        None | Some("") => "local-socket".to_owned(),
        Some(path) if path.starts_with('/') => path.to_owned(),
        Some(name) => name.trim_end_matches('.').to_ascii_lowercase(),
    }
}

fn scope_line(scope: KeychainScope, profile_file: &Path) -> String {
    match scope {
        KeychainScope::Target => "scope=target".to_owned(),
        KeychainScope::File => {
            let canonical =
                std::fs::canonicalize(profile_file).unwrap_or_else(|_| profile_file.to_path_buf());
            format!(
                "scope=file:{}",
                crate::audit::sha256_hex(canonical.to_string_lossy().as_bytes())
            )
        }
    }
}

fn account(kind: &str, endpoint: &str, record: &[String]) -> String {
    let digest: String = crate::audit::sha256_hex(record.join("\n").as_bytes())
        .chars()
        .take(DIGEST_HEX_CHARS)
        .collect();
    let readable: String = endpoint.chars().take(READABLE_CHARS).collect();
    format!("{kind}:{readable} #{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Resolved, SshTransport};
    use std::time::Duration;

    fn ssh(host: &str, jump: Vec<String>) -> SshSettings {
        SshSettings {
            host: Resolved::preset(host.to_owned()),
            port: Resolved::preset(22),
            user: Some(Resolved::preset("deploy".to_owned())),
            key_file: None,
            agent: Resolved::preset(true),
            password: None,
            trust_new_host: Resolved::preset(false),
            transport: Resolved::preset(SshTransport::InProcess),
            jump: Resolved::preset(jump),
            known_hosts: None,
            config_file: None,
            connect_timeout: Resolved::preset(Duration::from_secs(10)),
        }
    }

    fn target(host: &str) -> Target<'_> {
        Target {
            user: "alice",
            host: Some(host),
            port: 5432,
        }
    }

    #[test]
    fn a_password_account_is_readable_and_bound_to_the_file_and_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("profiles.toml");
        std::fs::write(&file, "").unwrap();
        let other = dir.path().join("other.toml");
        std::fs::write(&other, "").unwrap();
        let account = password_account(KeychainScope::File, &file, target("DB.example.com."), None);
        assert!(
            account.starts_with("pg:alice@db.example.com:5432 #"),
            "{account}"
        );
        assert_eq!(account.rsplit('#').next().map(str::len), Some(32));
        assert_eq!(
            account,
            password_account(KeychainScope::File, &file, target("db.example.com"), None)
        );
        for different in [
            password_account(KeychainScope::File, &other, target("db.example.com"), None),
            password_account(KeychainScope::File, &file, target("evil.example.com"), None),
            password_account(KeychainScope::Target, &file, target("db.example.com"), None),
            password_account(
                KeychainScope::File,
                &file,
                target("db.example.com"),
                Some(&ssh("bastion", Vec::new())),
            ),
        ] {
            assert_ne!(account, different);
        }
        assert_eq!(
            password_account(KeychainScope::Target, &file, target("db.example.com"), None),
            password_account(
                KeychainScope::Target,
                &other,
                target("db.example.com"),
                None
            )
        );
    }

    #[test]
    fn user_case_is_kept_so_distinct_roles_never_share_a_secret() {
        let file = Path::new("/nonexistent/profiles.toml");
        let lower = password_account(KeychainScope::Target, file, target("db"), None);
        let upper = password_account(
            KeychainScope::Target,
            file,
            Target {
                user: "Alice",
                ..target("db")
            },
            None,
        );
        assert_ne!(lower, upper);
    }

    #[test]
    fn an_ssh_account_is_bound_to_the_bastion_and_its_jumps() {
        let file = Path::new("/nonexistent/profiles.toml");
        let direct = ssh_secret_account(KeychainScope::Target, file, &ssh("bastion", Vec::new()));
        assert!(direct.starts_with("ssh:deploy@bastion:22 #"), "{direct}");
        let jumped = ssh_secret_account(
            KeychainScope::Target,
            file,
            &ssh("bastion", vec!["ops@jump:22".to_owned()]),
        );
        assert_ne!(direct, jumped);
    }
}
