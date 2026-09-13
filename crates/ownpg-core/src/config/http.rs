use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::environment::Environment;
use super::{Mode, Origin, Resolved};
use crate::error::{Error, Result};

pub const DEFAULT_BIND: &str = "127.0.0.1:8765";
pub const DEFAULT_BODY_CAP: usize = 1024 * 1024;
pub const DEFAULT_RATE_LIMIT: u32 = 60;
pub const DEFAULT_SHUTDOWN: Duration = Duration::from_secs(10);
pub const DEFAULT_POOL_SIZE: u32 = 4;
pub const MAX_POOL_SIZE: u32 = 64;
pub const DEFAULT_MAX_CONNECTIONS: u32 = 1_024;
pub const DEFAULT_HEADER_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_BODY_TIMEOUT: Duration = Duration::from_secs(30);
pub const MCP_PATH: &str = "/mcp";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    None,
    Bearer,
    Oauth,
}

impl AuthMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bearer => "bearer",
            Self::Oauth => "oauth",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "none" => Some(Self::None),
            "bearer" => Some(Self::Bearer),
            "oauth" => Some(Self::Oauth),
            _ => None,
        }
    }
}

impl std::fmt::Display for AuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_hosts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_origins: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_cap_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_client_sessions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shutdown_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_jwks_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_audience: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_key_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otel_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_proxies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OauthSettings {
    pub issuer: Resolved<String>,
    pub jwks_url: Resolved<String>,
    pub audience: Resolved<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSettings {
    None,
    Bearer {
        tokens_file: Option<Resolved<PathBuf>>,
        tokens_from_environment: bool,
    },
    Oauth(OauthSettings),
}

impl AuthSettings {
    #[must_use]
    pub const fn mode(&self) -> AuthMode {
        match self {
            Self::None => AuthMode::None,
            Self::Bearer { .. } => AuthMode::Bearer,
            Self::Oauth(_) => AuthMode::Oauth,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpSettings {
    pub enabled: bool,
    pub bind: Resolved<SocketAddr>,
    pub public_url: Resolved<String>,
    pub allowed_hosts: Resolved<Vec<String>>,
    pub allowed_origins: Resolved<Vec<String>>,
    pub body_cap: Resolved<usize>,
    pub rate_limit_per_minute: Resolved<u32>,
    pub older_client_sessions: Resolved<bool>,
    pub shutdown: Resolved<Duration>,
    pub pool_size: Resolved<u32>,
    pub auth: AuthSettings,
    pub auth_origin: Origin,
    pub state_key_file: Option<Resolved<PathBuf>>,
    pub otel_endpoint: Option<Resolved<String>>,
    pub trusted_proxies: Resolved<Vec<IpAddr>>,
    pub max_connections: Resolved<u32>,
}

impl HttpSettings {
    #[must_use]
    pub fn metadata_url(&self) -> String {
        match url::Url::parse(&self.public_url.value) {
            Ok(parsed) if parsed.host_str().is_some() => {
                let url_origin = url_origin_of(&self.public_url.value).unwrap_or_default();
                let path = parsed.path().trim_end_matches('/');
                format!("{url_origin}/.well-known/oauth-protected-resource{path}")
            }
            _ => format!(
                "{}/.well-known/oauth-protected-resource{MCP_PATH}",
                self.public_url.value
            ),
        }
    }

    #[must_use]
    pub fn is_loopback(&self) -> bool {
        self.bind.value.ip().is_loopback()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpFlags {
    pub enabled: bool,
    pub bind: Option<String>,
    pub auth: Option<AuthMode>,
}

fn invalid(setting: &str, value: &str, detail: &str) -> Error {
    Error::ConfigInvalid {
        setting: setting.to_owned(),
        value: value.to_owned(),
        detail: detail.to_owned(),
    }
}

fn parse_bind(setting: &str, text: &str) -> Result<SocketAddr> {
    let trimmed = text.trim();
    if let Ok(address) = trimmed.parse::<SocketAddr>() {
        return Ok(address);
    }
    let bare = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, 8765));
    }
    if let Some(port) = trimmed.strip_prefix(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return Ok(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port));
    }
    Err(invalid(
        setting,
        trimmed,
        "expected host:port such as 127.0.0.1:8765 or [::1]:8765",
    ))
}

fn parse_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_public_url(setting: &str, text: &str) -> Result<String> {
    let parsed = url::Url::parse(text.trim())
        .map_err(|error| invalid(setting, text, &format!("not a URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(invalid(setting, text, "the scheme must be http or https"));
    }
    if parsed.host_str().is_none() {
        return Err(invalid(setting, text, "the URL needs a host"));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(invalid(
            setting,
            text,
            "the URL must not carry a query or a fragment",
        ));
    }
    let mut canonical = parsed;
    let path = canonical.path().trim_end_matches('/').to_owned();
    canonical.set_path(if path.is_empty() {
        MCP_PATH
    } else if path.ends_with(MCP_PATH) {
        &path
    } else {
        return Err(invalid(setting, text, "the path must end with /mcp"));
    });
    Ok(canonical.to_string())
}

fn url_origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let default_port = match parsed.scheme() {
        "https" => 443,
        _ => 80,
    };
    let port = parsed.port().unwrap_or(default_port);
    let host_text = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Some(if port == default_port {
        format!("{}://{host_text}", parsed.scheme())
    } else {
        format!("{}://{host_text}:{port}", parsed.scheme())
    })
}

fn pick<T>(flag: Option<T>, env: Option<T>, profile: Option<T>) -> Option<Resolved<T>> {
    if let Some(value) = flag {
        return Some(Resolved::new(value, Origin::Flag));
    }
    if let Some(value) = env {
        return Some(Resolved::new(value, Origin::Environment));
    }
    profile.map(|value| Resolved::new(value, Origin::Profile))
}

fn env_u64(env: &Environment, name: &str) -> Result<Option<u64>> {
    let Some(raw) = env.var(name) else {
        return Ok(None);
    };
    raw.trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|_| invalid(name, raw, "expected a whole number"))
}

fn env_bool(env: &Environment, name: &str) -> Result<Option<bool>> {
    let Some(raw) = env.var(name) else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        other => Err(invalid(name, other, "expected true or false")),
    }
}

pub fn resolve_http(
    flags: &HttpFlags,
    env: &Environment,
    profile: Option<&HttpEntry>,
    mode: Mode,
) -> Result<HttpSettings> {
    let entry = profile.cloned().unwrap_or_default();
    let bind = pick(
        flags
            .bind
            .as_deref()
            .map(|text| parse_bind("bind", text))
            .transpose()?,
        env.var("OWNPG_BIND")
            .map(|text| parse_bind("OWNPG_BIND", text))
            .transpose()?,
        entry
            .bind
            .as_deref()
            .map(|text| parse_bind("http.bind", text))
            .transpose()?,
    )
    .unwrap_or_else(|| {
        Resolved::preset(
            DEFAULT_BIND
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 8765))),
        )
    });
    let public_url = pick(
        None,
        env.var("OWNPG_PUBLIC_URL")
            .map(|text| parse_public_url("OWNPG_PUBLIC_URL", text))
            .transpose()?,
        entry
            .public_url
            .as_deref()
            .map(|text| parse_public_url("http.public_url", text))
            .transpose()?,
    )
    .unwrap_or_else(|| Resolved::preset(format!("http://{}{MCP_PATH}", bind.value)));
    let mut default_hosts = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
        "[::1]".to_owned(),
    ];
    let bind_host = match bind.value.ip() {
        IpAddr::V6(ip) => format!("[{ip}]"),
        IpAddr::V4(ip) => ip.to_string(),
    };
    for candidate in [
        bind_host.clone(),
        format!("{bind_host}:{}", bind.value.port()),
        format!("localhost:{}", bind.value.port()),
        format!("127.0.0.1:{}", bind.value.port()),
        format!("[::1]:{}", bind.value.port()),
    ] {
        if !default_hosts.contains(&candidate) {
            default_hosts.push(candidate);
        }
    }
    if let Ok(parsed) = url::Url::parse(&public_url.value)
        && let Some(host) = parsed.host_str()
    {
        let host_text = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        for candidate in [
            host_text.clone(),
            parsed
                .port()
                .map(|port| format!("{host_text}:{port}"))
                .unwrap_or_else(|| host_text.clone()),
        ] {
            if !default_hosts.contains(&candidate) {
                default_hosts.push(candidate);
            }
        }
    }
    let allowed_hosts = pick(
        None,
        env.var("OWNPG_ALLOWED_HOSTS").map(parse_list),
        entry.allowed_hosts.clone(),
    )
    .filter(|resolved| !resolved.value.is_empty())
    .unwrap_or_else(|| Resolved::preset(default_hosts));
    let mut default_origins: Vec<String> = [
        format!("http://localhost:{}", bind.value.port()),
        format!("http://127.0.0.1:{}", bind.value.port()),
        format!("http://[::1]:{}", bind.value.port()),
    ]
    .into_iter()
    .collect();
    if let Some(origin) = url_origin_of(&public_url.value)
        && !default_origins.contains(&origin)
    {
        default_origins.push(origin);
    }
    let allowed_origins = pick(
        None,
        env.var("OWNPG_ALLOWED_ORIGINS").map(parse_list),
        entry.allowed_origins.clone(),
    )
    .filter(|resolved| !resolved.value.is_empty())
    .unwrap_or_else(|| Resolved::preset(default_origins));
    let body_cap = pick(
        None,
        env_u64(env, "OWNPG_BODY_CAP_BYTES")?,
        entry.body_cap_bytes,
    )
    .map(|resolved| {
        Resolved::new(
            usize::try_from(resolved.value).unwrap_or(usize::MAX),
            resolved.origin,
        )
    })
    .unwrap_or_else(|| Resolved::preset(DEFAULT_BODY_CAP));
    if body_cap.value < 1024 {
        return Err(invalid(
            "http.body_cap_bytes",
            &body_cap.value.to_string(),
            "the body cap must be at least 1024 bytes",
        ));
    }
    let rate_limit_per_minute = pick(
        None,
        env_u64(env, "OWNPG_RATE_LIMIT_PER_MINUTE")?
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
        entry.rate_limit_per_minute,
    )
    .unwrap_or_else(|| Resolved::preset(DEFAULT_RATE_LIMIT));
    if rate_limit_per_minute.value == 0 {
        return Err(invalid(
            "http.rate_limit_per_minute",
            "0",
            "the rate limit must allow at least one call per minute",
        ));
    }
    let older_client_sessions = pick(
        None,
        env_bool(env, "OWNPG_OLDER_CLIENT_SESSIONS")?,
        entry.older_client_sessions,
    )
    .unwrap_or_else(|| Resolved::preset(false));
    let shutdown = pick(
        None,
        env_u64(env, "OWNPG_SHUTDOWN_SECONDS")?,
        entry.shutdown_seconds,
    )
    .map(|resolved| Resolved::new(Duration::from_secs(resolved.value), resolved.origin))
    .unwrap_or_else(|| Resolved::preset(DEFAULT_SHUTDOWN));
    let pool_size = pick(
        None,
        env_u64(env, "OWNPG_POOL_SIZE")?.map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
        entry.pool_size,
    )
    .unwrap_or_else(|| Resolved::preset(DEFAULT_POOL_SIZE));
    if pool_size.value == 0 || pool_size.value > MAX_POOL_SIZE {
        return Err(invalid(
            "http.pool_size",
            &pool_size.value.to_string(),
            &format!("the pool size must be between 1 and {MAX_POOL_SIZE}"),
        ));
    }
    let auth_mode = pick(
        flags.auth,
        env.var("OWNPG_AUTH")
            .map(|text| {
                AuthMode::parse(text)
                    .ok_or_else(|| invalid("OWNPG_AUTH", text, "expected none, bearer, or oauth"))
            })
            .transpose()?,
        entry.auth,
    );
    let (auth_mode, auth_origin) = match auth_mode {
        Some(resolved) => (resolved.value, resolved.origin),
        None => (AuthMode::None, Origin::Preset),
    };
    let auth = match auth_mode {
        AuthMode::None => AuthSettings::None,
        AuthMode::Bearer => {
            let tokens_file = pick(
                None,
                env.var("OWNPG_BEARER_TOKENS_FILE").map(PathBuf::from),
                entry.tokens_file.clone(),
            );
            let tokens_from_environment = env.var("OWNPG_BEARER_TOKENS").is_some();
            if tokens_file.is_none() && !tokens_from_environment {
                return Err(invalid(
                    "auth",
                    "bearer",
                    "bearer mode needs OWNPG_BEARER_TOKENS_FILE (or http.tokens_file) or OWNPG_BEARER_TOKENS",
                ));
            }
            AuthSettings::Bearer {
                tokens_file,
                tokens_from_environment,
            }
        }
        AuthMode::Oauth => {
            let issuer = pick(
                None,
                env.var("OWNPG_OAUTH_ISSUER").map(str::to_owned),
                entry.oauth_issuer.clone(),
            )
            .ok_or_else(|| {
                invalid(
                    "auth",
                    "oauth",
                    "oauth mode needs OWNPG_OAUTH_ISSUER (or http.oauth_issuer)",
                )
            })?;
            let issuer_url = url::Url::parse(issuer.value.trim()).map_err(|error| {
                invalid(
                    "http.oauth_issuer",
                    &issuer.value,
                    &format!("not a URL: {error}"),
                )
            })?;
            if issuer_url.scheme() != "https" {
                return Err(invalid(
                    "http.oauth_issuer",
                    &issuer.value,
                    "the issuer must use https",
                ));
            }
            let jwks_url = pick(
                None,
                env.var("OWNPG_OAUTH_JWKS_URL").map(str::to_owned),
                entry.oauth_jwks_url.clone(),
            )
            .unwrap_or_else(|| {
                Resolved::preset(format!(
                    "{}/.well-known/jwks.json",
                    issuer.value.trim_end_matches('/')
                ))
            });
            let jwks_parsed = url::Url::parse(jwks_url.value.trim()).map_err(|error| {
                invalid(
                    "http.oauth_jwks_url",
                    &jwks_url.value,
                    &format!("not a URL: {error}"),
                )
            })?;
            let jwks_loopback = jwks_parsed.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
            if jwks_parsed.scheme() != "https" && !(jwks_parsed.scheme() == "http" && jwks_loopback)
            {
                return Err(invalid(
                    "http.oauth_jwks_url",
                    &jwks_url.value,
                    "the JWKS URL must use https unless it points at the loopback interface",
                ));
            }
            let audience = pick(
                None,
                env.var("OWNPG_OAUTH_AUDIENCE").map(str::to_owned),
                entry.oauth_audience.clone(),
            )
            .unwrap_or_else(|| Resolved::preset(public_url.value.clone()));
            AuthSettings::Oauth(OauthSettings {
                issuer: Resolved::new(
                    issuer.value.trim().trim_end_matches('/').to_owned(),
                    issuer.origin,
                ),
                jwks_url,
                audience,
            })
        }
    };
    let state_key_file = pick(
        None,
        env.var("OWNPG_STATE_KEY_FILE").map(PathBuf::from),
        entry.state_key_file.clone(),
    );
    let otel_endpoint = pick(
        None,
        env.var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        entry.otel_endpoint.clone(),
    );
    let trusted_proxies = pick(
        None,
        env.var("OWNPG_TRUSTED_PROXIES").map(parse_list),
        entry.trusted_proxies.clone(),
    )
    .map(|resolved| {
        let parsed: Result<Vec<IpAddr>> = resolved
            .value
            .iter()
            .map(|text| {
                text.trim()
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .map_err(|_| {
                        invalid(
                            "http.trusted_proxies",
                            text,
                            "expected an IP address of a proxy this server sits behind",
                        )
                    })
            })
            .collect();
        parsed.map(|value| Resolved::new(value, resolved.origin))
    })
    .transpose()?
    .unwrap_or_else(|| Resolved::preset(Vec::new()));
    let max_connections = pick(
        None,
        env_u64(env, "OWNPG_MAX_CONNECTIONS")?
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
        entry.max_connections,
    )
    .unwrap_or_else(|| Resolved::preset(DEFAULT_MAX_CONNECTIONS));
    if max_connections.value == 0 {
        return Err(invalid(
            "http.max_connections",
            "0",
            "the connection cap must allow at least one connection",
        ));
    }
    let settings = HttpSettings {
        enabled: flags.enabled,
        bind,
        public_url,
        allowed_hosts,
        allowed_origins,
        body_cap,
        rate_limit_per_minute,
        older_client_sessions,
        shutdown,
        pool_size,
        auth,
        auth_origin,
        state_key_file,
        otel_endpoint,
        trusted_proxies,
        max_connections,
    };
    if settings.enabled && settings.auth.mode() == AuthMode::None {
        if !settings.is_loopback() {
            return Err(invalid(
                "bind",
                &settings.bind.value.to_string(),
                "a bind address outside the loopback interface needs --auth bearer or --auth oauth",
            ));
        }
        if let Some(host) = url::Url::parse(&settings.public_url.value)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            && !is_local_host(&host)
        {
            return Err(invalid(
                "http.public_url",
                &settings.public_url.value,
                "a public URL outside the loopback interface needs --auth bearer or --auth oauth",
            ));
        }
        if let Some(host) = settings
            .allowed_hosts
            .value
            .iter()
            .find(|host| !is_local_host(host))
        {
            return Err(invalid(
                "http.allowed_hosts",
                host,
                "a host outside the loopback interface needs --auth bearer or --auth oauth",
            ));
        }
        if !mode.allows_writes() {
            tracing::debug!("HTTP without authentication on loopback in a read mode");
        }
    }
    if settings.enabled
        && !settings.is_loopback()
        && settings.public_url.value.starts_with("http://")
    {
        tracing::warn!(
            public_url = %settings.public_url.value,
            "the public URL uses plain http, so bearer tokens travel in clear text; put TLS in front of this server"
        );
    }
    Ok(settings)
}

fn is_local_host(host: &str) -> bool {
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host.rsplit_once(':')
            .filter(|(name, port)| {
                !name.contains(':') && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())
            })
            .map_or(host, |(name, _)| name)
    };
    bare.eq_ignore_ascii_case("localhost")
        || bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Environment {
        Environment::new(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            None,
            None,
        )
    }

    #[test]
    fn the_defaults_bind_loopback_without_authentication() {
        let flags = HttpFlags {
            enabled: true,
            ..HttpFlags::default()
        };
        let settings = resolve_http(&flags, &env(&[]), None, Mode::ReadOnly).unwrap();
        assert_eq!(settings.bind.value.to_string(), "127.0.0.1:8765");
        assert_eq!(settings.public_url.value, "http://127.0.0.1:8765/mcp");
        assert_eq!(settings.auth, AuthSettings::None);
        assert_eq!(settings.body_cap.value, DEFAULT_BODY_CAP);
        assert_eq!(settings.rate_limit_per_minute.value, 60);
        assert!(!settings.older_client_sessions.value);
        assert!(
            settings
                .allowed_origins
                .value
                .contains(&"http://127.0.0.1:8765".to_owned())
        );
        assert!(
            settings
                .allowed_hosts
                .value
                .contains(&"127.0.0.1:8765".to_owned())
        );
        assert_eq!(
            settings.metadata_url(),
            "http://127.0.0.1:8765/.well-known/oauth-protected-resource/mcp"
        );
        assert!(settings.trusted_proxies.value.is_empty());
        assert_eq!(settings.max_connections.value, DEFAULT_MAX_CONNECTIONS);
    }

    #[test]
    fn trusted_proxies_parse_as_addresses_and_the_metadata_url_sits_at_the_root() {
        let flags = HttpFlags {
            enabled: true,
            bind: Some("0.0.0.0:8765".to_owned()),
            auth: Some(AuthMode::Bearer),
        };
        let settings = resolve_http(
            &flags,
            &env(&[
                (
                    "OWNPG_BEARER_TOKENS",
                    "reader-token-0123456789abcdef read-only",
                ),
                ("OWNPG_TRUSTED_PROXIES", "10.0.0.5, [::1]"),
                ("OWNPG_PUBLIC_URL", "https://db.example.com:8443/api/mcp"),
                ("OWNPG_MAX_CONNECTIONS", "64"),
            ]),
            None,
            Mode::ReadOnly,
        )
        .unwrap();
        assert_eq!(settings.trusted_proxies.value.len(), 2);
        assert_eq!(settings.max_connections.value, 64);
        assert_eq!(
            settings.metadata_url(),
            "https://db.example.com:8443/.well-known/oauth-protected-resource/api/mcp"
        );
        let error = resolve_http(
            &flags,
            &env(&[
                (
                    "OWNPG_BEARER_TOKENS",
                    "reader-token-0123456789abcdef read-only",
                ),
                ("OWNPG_TRUSTED_PROXIES", "proxy.internal"),
            ]),
            None,
            Mode::ReadOnly,
        )
        .unwrap_err();
        assert!(error.to_string().contains("proxy"), "{error}");
    }

    #[test]
    fn a_public_bind_needs_an_auth_mode() {
        let flags = HttpFlags {
            enabled: true,
            bind: Some("0.0.0.0:9000".to_owned()),
            auth: None,
        };
        let error = resolve_http(&flags, &env(&[]), None, Mode::ReadOnly).unwrap_err();
        assert!(error.to_string().contains("--auth"), "{error}");
        let flags = HttpFlags {
            enabled: true,
            bind: Some("0.0.0.0:9000".to_owned()),
            auth: Some(AuthMode::Bearer),
        };
        let settings = resolve_http(
            &flags,
            &env(&[("OWNPG_BEARER_TOKENS", "abc read-only")]),
            None,
            Mode::ReadOnly,
        )
        .unwrap();
        assert_eq!(settings.auth.mode(), AuthMode::Bearer);
        assert_eq!(settings.auth_origin, Origin::Flag);
    }

    #[test]
    fn oauth_settings_derive_the_jwks_url_and_audience() {
        let flags = HttpFlags {
            enabled: true,
            bind: Some("0.0.0.0:443".to_owned()),
            auth: Some(AuthMode::Oauth),
        };
        let settings = resolve_http(
            &flags,
            &env(&[
                ("OWNPG_OAUTH_ISSUER", "https://auth.example.com/"),
                ("OWNPG_PUBLIC_URL", "https://db.example.com"),
            ]),
            None,
            Mode::ReadWrite,
        )
        .unwrap();
        let AuthSettings::Oauth(oauth) = &settings.auth else {
            panic!("oauth expected");
        };
        assert_eq!(oauth.issuer.value, "https://auth.example.com");
        assert_eq!(
            oauth.jwks_url.value,
            "https://auth.example.com/.well-known/jwks.json"
        );
        assert_eq!(oauth.audience.value, "https://db.example.com/mcp");
        assert_eq!(settings.public_url.value, "https://db.example.com/mcp");
        assert!(
            settings
                .allowed_origins
                .value
                .contains(&"https://db.example.com".to_owned())
        );
        assert!(
            settings
                .allowed_hosts
                .value
                .contains(&"db.example.com".to_owned())
        );
        let error = resolve_http(
            &flags,
            &env(&[("OWNPG_OAUTH_ISSUER", "http://auth.example.com")]),
            None,
            Mode::ReadWrite,
        )
        .unwrap_err();
        assert!(error.to_string().contains("https"), "{error}");
    }

    #[test]
    fn a_public_url_or_host_outside_loopback_needs_authentication() {
        let flags = HttpFlags {
            enabled: true,
            ..HttpFlags::default()
        };
        let error = resolve_http(
            &flags,
            &env(&[("OWNPG_PUBLIC_URL", "https://db.example.com/mcp")]),
            None,
            Mode::ReadOnly,
        )
        .unwrap_err();
        assert!(error.to_string().contains("--auth"), "{error}");
        let error = resolve_http(
            &flags,
            &env(&[("OWNPG_ALLOWED_HOSTS", "localhost,db.example.com")]),
            None,
            Mode::ReadOnly,
        )
        .unwrap_err();
        assert!(error.to_string().contains("db.example.com"), "{error}");
        let settings = resolve_http(
            &flags,
            &env(&[("OWNPG_ALLOWED_HOSTS", "localhost:8765,[::1]:8765,127.0.0.1")]),
            None,
            Mode::ReadOnly,
        )
        .unwrap();
        assert_eq!(settings.allowed_hosts.value.len(), 3);
        assert!(is_local_host("[::1]:8765"));
        assert!(is_local_host("LOCALHOST"));
        assert!(!is_local_host("db.example.com:443"));
    }

    #[test]
    fn an_empty_host_or_origin_list_keeps_the_defaults() {
        let flags = HttpFlags {
            enabled: true,
            ..HttpFlags::default()
        };
        let settings = resolve_http(
            &flags,
            &env(&[
                ("OWNPG_ALLOWED_HOSTS", ","),
                ("OWNPG_ALLOWED_ORIGINS", " , "),
            ]),
            None,
            Mode::ReadOnly,
        )
        .unwrap();
        assert!(!settings.allowed_hosts.value.is_empty());
        assert!(!settings.allowed_origins.value.is_empty());
        assert_eq!(settings.allowed_hosts.origin, Origin::Preset);
        let entry = HttpEntry {
            allowed_hosts: Some(Vec::new()),
            ..HttpEntry::default()
        };
        let settings = resolve_http(&flags, &env(&[]), Some(&entry), Mode::ReadOnly).unwrap();
        assert!(!settings.allowed_hosts.value.is_empty());
    }

    #[test]
    fn bind_accepts_a_bare_port_and_ip() {
        assert_eq!(
            parse_bind("bind", ":9999").unwrap().to_string(),
            "127.0.0.1:9999"
        );
        assert_eq!(
            parse_bind("bind", "[::1]").unwrap().to_string(),
            "[::1]:8765"
        );
        assert!(parse_bind("bind", "nonsense").is_err());
        assert_eq!(
            parse_public_url("x", "https://db.example.com/").unwrap(),
            "https://db.example.com/mcp"
        );
        assert!(parse_public_url("x", "https://db.example.com/other").is_err());
        assert!(parse_public_url("x", "https://db.example.com/mcp?x=1").is_err());
        assert_eq!(
            url_origin_of("https://db.example.com:8443/mcp").as_deref(),
            Some("https://db.example.com:8443")
        );
        assert_eq!(
            url_origin_of("https://db.example.com/mcp").as_deref(),
            Some("https://db.example.com")
        );
    }
}
