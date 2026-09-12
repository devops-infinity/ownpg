use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::{Origin, Resolved, Settings};

pub const MASKED: &str = "set";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct SettingLine {
    pub name: String,
    pub value: String,
    pub origin: String,
}

impl SettingLine {
    fn new(name: &str, value: impl Into<String>, origin: Origin) -> Self {
        Self {
            name: name.to_owned(),
            value: value.into(),
            origin: origin.label().to_owned(),
        }
    }

    fn resolved<T: ToString>(name: &str, resolved: &Resolved<T>) -> Self {
        Self::new(name, resolved.value.to_string(), resolved.origin)
    }

    fn optional<T: ToString>(name: &str, resolved: Option<&Resolved<T>>) -> Self {
        match resolved {
            Some(resolved) => Self::resolved(name, resolved),
            None => Self::new(name, "unset", Origin::Preset),
        }
    }

    fn path(name: &str, resolved: Option<&Resolved<std::path::PathBuf>>) -> Self {
        match resolved {
            Some(resolved) => {
                Self::new(name, resolved.value.display().to_string(), resolved.origin)
            }
            None => Self::new(name, "unset", Origin::Preset),
        }
    }

    fn duration(name: &str, resolved: &Resolved<Duration>) -> Self {
        Self::new(
            name,
            format!("{} ms", resolved.value.as_millis()),
            resolved.origin,
        )
    }

    fn secret<T>(name: &str, resolved: Option<&Resolved<T>>) -> Self {
        match resolved {
            Some(resolved) => Self::new(name, MASKED, resolved.origin),
            None => Self::new(name, "unset", Origin::Preset),
        }
    }
}

#[must_use]
pub fn describe(settings: &Settings) -> Vec<SettingLine> {
    let connection = &settings.connection;
    let limits = &settings.limits;
    let mut lines = vec![
        SettingLine::new(
            "profile",
            settings
                .profile
                .clone()
                .unwrap_or_else(|| "none".to_owned()),
            if settings.profile.is_some() {
                Origin::Flag
            } else {
                Origin::Preset
            },
        ),
        SettingLine::resolved("mode", &settings.mode),
        SettingLine::resolved("database", &settings.database),
        SettingLine::resolved("schema", &settings.schema),
        SettingLine::secret("dsn", connection.dsn.as_ref()),
        SettingLine::optional("service", connection.service.as_ref()),
        SettingLine::optional("host", connection.host.as_ref()),
        SettingLine::optional("hostaddr", connection.hostaddr.as_ref()),
        SettingLine::resolved("port", &connection.port),
        SettingLine::resolved("user", &connection.user),
        SettingLine::secret("password", connection.password.as_ref()),
        SettingLine::resolved("sslmode", &connection.sslmode),
        SettingLine::path("sslrootcert", connection.sslrootcert.as_ref()),
        SettingLine::path("sslcert", connection.sslcert.as_ref()),
        SettingLine::path("sslkey", connection.sslkey.as_ref()),
        SettingLine::resolved("channel_binding", &connection.channel_binding),
        SettingLine::duration("connect_timeout", &connection.connect_timeout),
        SettingLine::resolved("application_name", &connection.application_name),
        SettingLine::optional("options", connection.options.as_ref()),
        SettingLine::new(
            "pooled",
            connection
                .pooled
                .value
                .map_or_else(|| "auto".to_owned(), |pooled| pooled.to_string()),
            connection.pooled.origin,
        ),
        SettingLine::duration("statement_timeout", &limits.statement_timeout),
        SettingLine::duration("lock_timeout", &limits.lock_timeout),
        SettingLine::duration("transaction_timeout", &limits.transaction_timeout),
        SettingLine::new(
            "handle_expiry",
            format!("{} s", limits.handle_expiry.value.as_secs()),
            limits.handle_expiry.origin,
        ),
        SettingLine::resolved("row_cap", &limits.row_cap),
        SettingLine::resolved("byte_cap", &limits.byte_cap),
        SettingLine::resolved("strict_role", &settings.strict_role),
        SettingLine::new(
            "tools",
            if settings.tools.value.is_empty() {
                "none".to_owned()
            } else {
                settings
                    .tools
                    .value
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            },
            settings.tools.origin,
        ),
    ];
    match &settings.ssh {
        Some(ssh) => {
            lines.push(SettingLine::resolved("ssh.host", &ssh.host));
            lines.push(SettingLine::resolved("ssh.port", &ssh.port));
            lines.push(SettingLine::optional("ssh.user", ssh.user.as_ref()));
            lines.push(SettingLine::path("ssh.key_file", ssh.key_file.as_ref()));
            lines.push(SettingLine::resolved("ssh.agent", &ssh.agent));
            lines.push(SettingLine::secret("ssh.password", ssh.password.as_ref()));
            lines.push(SettingLine::resolved(
                "ssh.trust_new_host",
                &ssh.trust_new_host,
            ));
            lines.push(SettingLine::resolved("ssh.transport", &ssh.transport));
            lines.push(SettingLine::new(
                "ssh.jump",
                if ssh.jump.value.is_empty() {
                    "none".to_owned()
                } else {
                    ssh.jump.value.join(",")
                },
                ssh.jump.origin,
            ));
            lines.push(SettingLine::path(
                "ssh.known_hosts",
                ssh.known_hosts.as_ref(),
            ));
            lines.push(SettingLine::path(
                "ssh.config_file",
                ssh.config_file.as_ref(),
            ));
            lines.push(SettingLine::duration(
                "ssh.connect_timeout",
                &ssh.connect_timeout,
            ));
        }
        None => lines.push(SettingLine::new("ssh", "off", Origin::Preset)),
    }
    lines.push(SettingLine::resolved("audit", &settings.audit.enabled));
    lines.push(SettingLine::path(
        "audit_path",
        settings.audit.path.as_ref(),
    ));
    lines.push(SettingLine::resolved(
        "audit_max_bytes",
        &settings.audit.max_bytes,
    ));
    lines.push(SettingLine::path("pg_bindir", settings.pg_bindir.as_ref()));
    lines.push(SettingLine::path(
        "output_dir",
        settings.output_dir.as_ref(),
    ));
    lines.push(SettingLine::resolved("no_input", &settings.no_input));
    let http = &settings.http;
    lines.push(SettingLine::resolved("http.bind", &http.bind));
    lines.push(SettingLine::resolved("http.public_url", &http.public_url));
    lines.push(SettingLine::new(
        "http.allowed_hosts",
        http.allowed_hosts.value.join(","),
        http.allowed_hosts.origin,
    ));
    lines.push(SettingLine::new(
        "http.allowed_origins",
        http.allowed_origins.value.join(","),
        http.allowed_origins.origin,
    ));
    lines.push(SettingLine::resolved("http.body_cap_bytes", &http.body_cap));
    lines.push(SettingLine::resolved(
        "http.rate_limit_per_minute",
        &http.rate_limit_per_minute,
    ));
    lines.push(SettingLine::resolved(
        "http.legacy_session_mode",
        &http.legacy_session_mode,
    ));
    lines.push(SettingLine::duration("http.shutdown", &http.shutdown));
    lines.push(SettingLine::resolved("http.pool_size", &http.pool_size));
    lines.push(SettingLine::new(
        "http.auth",
        http.auth.mode().as_str(),
        http.auth_origin,
    ));
    match &http.auth {
        crate::config::AuthSettings::Bearer {
            tokens_file,
            tokens_from_environment,
        } => {
            lines.push(SettingLine::path("http.tokens_file", tokens_file.as_ref()));
            lines.push(SettingLine::new(
                "http.tokens_from_environment",
                tokens_from_environment.to_string(),
                Origin::Environment,
            ));
        }
        crate::config::AuthSettings::Oauth(oauth) => {
            lines.push(SettingLine::resolved("http.oauth_issuer", &oauth.issuer));
            lines.push(SettingLine::resolved(
                "http.oauth_jwks_url",
                &oauth.jwks_url,
            ));
            lines.push(SettingLine::resolved(
                "http.oauth_audience",
                &oauth.audience,
            ));
        }
        crate::config::AuthSettings::None => {}
    }
    lines.push(SettingLine::path(
        "http.state_key_file",
        http.state_key_file.as_ref(),
    ));
    lines.push(SettingLine::optional(
        "http.otel_endpoint",
        http.otel_endpoint.as_ref(),
    ));
    lines.push(SettingLine::new(
        "config_file",
        display(&settings.paths.config_file),
        Origin::Preset,
    ));
    lines.push(SettingLine::new(
        "data_dir",
        display(&settings.paths.data_dir),
        Origin::Preset,
    ));
    lines.push(SettingLine::new(
        "cache_dir",
        display(&settings.paths.cache_dir),
        Origin::Preset,
    ));
    lines
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::{AppPaths, Environment, FlagLayer, Sources, resolve};

    #[test]
    fn secrets_are_masked_and_every_line_names_an_origin() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::new(
            BTreeMap::from([
                ("OWNPG_DATABASE".to_owned(), "app".to_owned()),
                ("PGPASSWORD".to_owned(), "hunter2".to_owned()),
            ]),
            Some(dir.path().to_path_buf()),
            Some("tester".to_owned()),
        );
        let paths = AppPaths::from_base(
            dir.path().join("c"),
            dir.path().join("d"),
            dir.path().join("k"),
        );
        let (settings, _) = resolve(
            FlagLayer::default(),
            Sources {
                env: &env,
                paths,
                keychain: None,
            },
        )
        .unwrap();
        let lines = describe(&settings);
        let rendered = serde_json::to_string(&lines).unwrap();
        assert!(!rendered.contains("hunter2"));
        let password = lines.iter().find(|line| line.name == "password").unwrap();
        assert_eq!(password.value, MASKED);
        assert!(lines.iter().all(|line| !line.origin.is_empty()));
        let database = lines.iter().find(|line| line.name == "database").unwrap();
        assert_eq!(database.value, "app");
        assert_eq!(database.origin, "environment");
    }
}
