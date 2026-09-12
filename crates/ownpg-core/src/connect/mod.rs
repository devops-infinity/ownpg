pub mod role;
pub mod ssh;
pub mod tls;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio_postgres::tls::MakeTlsConnect;
use tokio_postgres::{CancelToken, Client};

use crate::config::presets::{self, FALLBACK_TCP_HOST, FALLBACK_TCP_USER};
use crate::config::{ChannelBinding, Origin, Settings, SslMode};
use crate::error::{Error, Result};

pub use tls::{Tls, TlsPlan, Verification};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Socket { directory: PathBuf, port: u16 },
    Tcp { host: String, port: u16 },
}

impl Endpoint {
    #[must_use]
    pub fn is_socket(&self) -> bool {
        matches!(self, Self::Socket { .. })
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket { directory, port } => {
                write!(f, "{}", presets::socket_path(directory, *port).display())
            }
            Self::Tcp { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub endpoint: Endpoint,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub target: String,
    pub user: String,
    pub outcome: std::result::Result<(), String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Socket,
    Tcp,
    Ssh,
}

impl Via {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Socket => "unix socket",
            Self::Tcp => "tcp",
            Self::Ssh => "ssh tunnel",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub target: String,
    pub via: Via,
    pub endpoint: Option<Endpoint>,
    pub user: String,
    pub tls: Option<TlsPlan>,
    pub tls_used: Option<bool>,
    pub server_version: String,
    pub server_version_num: i32,
    pub role: String,
    pub database: String,
    pub search_path: String,
    pub pooled: bool,
    pub attempts: Vec<Attempt>,
}

pub struct Session {
    pub client: Client,
    pub cancel: CancelToken,
    pub info: SessionInfo,
    tls: Arc<Tls>,
    driver: tokio::task::JoinHandle<()>,
    keep: Vec<Box<dyn std::any::Any + Send + Sync>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("info", &self.info).finish()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

impl Session {
    pub async fn is_alive(&self) -> bool {
        !self.client.is_closed() && self.client.check_connection().await.is_ok()
    }

    pub async fn cancel_running_statement(&self) -> Result<()> {
        self.cancel
            .cancel_query(self.tls.connector.clone())
            .await
            .map_err(|error| Error::ProtocolFailed {
                detail: format!("the cancel request failed: {error}"),
            })
    }
}

pin_project_lite::pin_project! {
    #[project = TunnelStreamProj]
    pub enum TunnelStream {
        Channel { #[pin] inner: russh::ChannelStream<russh::client::Msg> },
        #[cfg(unix)]
        Unix { #[pin] inner: tokio::net::UnixStream },
    }
}

impl std::fmt::Debug for TunnelStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel { .. } => f.write_str("TunnelStream::Channel"),
            #[cfg(unix)]
            Self::Unix { .. } => f.write_str("TunnelStream::Unix"),
        }
    }
}

impl AsyncRead for TunnelStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            TunnelStreamProj::Channel { inner } => inner.poll_read(cx, buf),
            #[cfg(unix)]
            TunnelStreamProj::Unix { inner } => inner.poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TunnelStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.project() {
            TunnelStreamProj::Channel { inner } => inner.poll_write(cx, buf),
            #[cfg(unix)]
            TunnelStreamProj::Unix { inner } => inner.poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            TunnelStreamProj::Channel { inner } => inner.poll_flush(cx),
            #[cfg(unix)]
            TunnelStreamProj::Unix { inner } => inner.poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            TunnelStreamProj::Channel { inner } => inner.poll_shutdown(cx),
            #[cfg(unix)]
            TunnelStreamProj::Unix { inner } => inner.poll_shutdown(cx),
        }
    }
}

#[derive(Clone)]
pub struct Connector {
    settings: Arc<Settings>,
    ssh_hints: ssh::Hints,
}

impl fmt::Debug for Connector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connector").finish_non_exhaustive()
    }
}

impl Connector {
    #[must_use]
    pub fn new(settings: Arc<Settings>) -> Self {
        Self {
            settings,
            ssh_hints: ssh::Hints::default(),
        }
    }

    #[must_use]
    pub fn with_ssh_hints(mut self, hints: ssh::Hints) -> Self {
        self.ssh_hints = hints;
        self
    }

    #[must_use]
    pub fn candidates(&self) -> Vec<Candidate> {
        let connection = &self.settings.connection;
        let port = connection.port.value;
        let user = connection.user.value.clone();
        if let Some(host) = &connection.host {
            let endpoint = if presets::is_socket_directory(&host.value) {
                Endpoint::Socket {
                    directory: PathBuf::from(&host.value),
                    port,
                }
            } else {
                Endpoint::Tcp {
                    host: host.value.clone(),
                    port,
                }
            };
            return vec![Candidate { endpoint, user }];
        }
        if let Some(hostaddr) = &connection.hostaddr {
            return vec![Candidate {
                endpoint: Endpoint::Tcp {
                    host: hostaddr.value.clone(),
                    port,
                },
                user,
            }];
        }
        let mut candidates: Vec<Candidate> = presets::socket_directories()
            .into_iter()
            .map(|directory| Candidate {
                endpoint: Endpoint::Socket { directory, port },
                user: user.clone(),
            })
            .collect();
        let fallback_user = if connection.user.origin == Origin::Preset {
            FALLBACK_TCP_USER.to_owned()
        } else {
            user
        };
        candidates.push(Candidate {
            endpoint: Endpoint::Tcp {
                host: FALLBACK_TCP_HOST.to_owned(),
                port,
            },
            user: fallback_user,
        });
        candidates
    }

    pub fn driver_config(
        &self,
        candidate: &Candidate,
        ssl_mode: SslMode,
    ) -> tokio_postgres::Config {
        let settings = &self.settings;
        let connection = &settings.connection;
        let mut config = tokio_postgres::Config::new();
        match &candidate.endpoint {
            Endpoint::Socket { directory, port } => {
                config.host_path(directory);
                config.port(*port);
                config.ssl_mode(tokio_postgres::config::SslMode::Disable);
            }
            Endpoint::Tcp { host, port } => {
                config.host(host);
                config.port(*port);
                config.ssl_mode(match ssl_mode {
                    SslMode::Disable => tokio_postgres::config::SslMode::Disable,
                    SslMode::Allow | SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
                    SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                        tokio_postgres::config::SslMode::Require
                    }
                });
            }
        }
        if let Some(hostaddr) = &connection.hostaddr
            && connection.host.is_some()
            && let Ok(address) = hostaddr.value.parse()
        {
            config.hostaddr(address);
        }
        config.user(&candidate.user);
        config.dbname(&settings.database.value);
        if let Some(password) = &connection.password {
            config.password(password.value.expose());
        }
        config.channel_binding(match connection.channel_binding.value {
            ChannelBinding::Disable => tokio_postgres::config::ChannelBinding::Disable,
            ChannelBinding::Prefer => tokio_postgres::config::ChannelBinding::Prefer,
            ChannelBinding::Require => tokio_postgres::config::ChannelBinding::Require,
        });
        config.connect_timeout(connection.connect_timeout.value);
        config.application_name(&connection.application_name.value);
        config.keepalives(true);
        let mut options = Vec::new();
        if settings.connection.pooled.value != Some(true) {
            options.extend(
                session_settings(settings, None)
                    .into_iter()
                    .map(|(name, value)| format!("-c {name}={value}")),
            );
        }
        if let Some(extra) = &connection.options {
            options.push(extra.value.clone());
        }
        if !options.is_empty() {
            config.options(options.join(" "));
        }
        config
    }

    pub async fn connect(&self) -> Result<Session> {
        if let Some(ssh) = &self.settings.ssh {
            return self.connect_through_ssh(ssh).await;
        }
        let candidates = self.candidates();
        let mut attempts = Vec::new();
        let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
        for candidate in candidates {
            let target = candidate.endpoint.to_string();
            let tls = Arc::new(tls::build(&self.settings.connection, &target)?);
            match self.connect_candidate(&candidate, &tls).await {
                Ok(mut session) => {
                    attempts.push(Attempt {
                        target,
                        user: candidate.user.clone(),
                        outcome: Ok(()),
                    });
                    session.info.attempts = attempts;
                    return Ok(session);
                }
                Err(error) => {
                    attempts.push(Attempt {
                        target,
                        user: candidate.user.clone(),
                        outcome: Err(error.to_string()),
                    });
                    last_error = Some(error);
                }
            }
        }
        Err(Error::ConnectFailed {
            tried: attempts
                .iter()
                .map(|attempt| format!("{} as {}", attempt.target, attempt.user))
                .collect(),
            source: last_error.unwrap_or_else(|| "no connection candidate".into()),
        })
    }

    async fn connect_candidate(
        &self,
        candidate: &Candidate,
        tls: &Arc<Tls>,
    ) -> std::result::Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        let requested = self.settings.connection.sslmode.value;
        let first = if requested == SslMode::Allow {
            SslMode::Disable
        } else {
            requested
        };
        match self.connect_once(candidate, tls, first).await {
            Ok(session) => Ok(session),
            Err(error) if requested == SslMode::Allow && !candidate.endpoint.is_socket() => {
                tracing::debug!(%error, "retrying with TLS after the plain attempt was refused");
                self.connect_once(candidate, tls, SslMode::Prefer).await
            }
            Err(error) => Err(error),
        }
    }

    async fn connect_once(
        &self,
        candidate: &Candidate,
        tls: &Arc<Tls>,
        ssl_mode: SslMode,
    ) -> std::result::Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        let config = self.driver_config(candidate, ssl_mode);
        let (client, connection) = config.connect(tls.connector.clone()).await?;
        let driver = tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "the database connection ended");
            }
        });
        let via = if candidate.endpoint.is_socket() {
            Via::Socket
        } else {
            Via::Tcp
        };
        self.finish(
            client,
            driver,
            tls,
            candidate.endpoint.to_string(),
            via,
            Some(candidate.endpoint.clone()),
            candidate.user.clone(),
        )
        .await
    }

    async fn connect_through_ssh(&self, ssh: &crate::config::SshSettings) -> Result<Session> {
        let candidate = self
            .candidates()
            .into_iter()
            .find(|candidate| !candidate.endpoint.is_socket())
            .ok_or_else(|| Error::SshFailed {
                host: ssh.host.value.clone(),
                detail: "a tunnel needs a TCP host and port on the far side; a socket directory cannot be forwarded".to_owned(),
                source: None,
            })?;
        let Endpoint::Tcp {
            host: target_host,
            port: target_port,
        } = candidate.endpoint
        else {
            return Err(Error::SshFailed {
                host: ssh.host.value.clone(),
                detail: "a tunnel needs a TCP host and port on the far side".to_owned(),
                source: None,
            });
        };
        let tunnel = ssh::open(ssh, &target_host, target_port, &self.ssh_hints).await?;
        let (stream, route, keep) = tunnel.into_parts();
        let target_name = format!("{target_host}:{target_port} via {}", route.join(" -> "));
        let mut session = self.connect_over(stream, &target_name, Via::Ssh).await?;
        session.keep = keep;
        Ok(session)
    }

    pub async fn connect_over<S>(&self, stream: S, target_name: &str, via: Via) -> Result<Session>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let candidate =
            self.candidates()
                .into_iter()
                .next()
                .ok_or_else(|| Error::ConnectFailed {
                    tried: Vec::new(),
                    source: "no connection candidate".into(),
                })?;
        let tls = Arc::new(tls::build(&self.settings.connection, target_name)?);
        let config = self.driver_config(&candidate, self.settings.connection.sslmode.value);
        let host_name = match &candidate.endpoint {
            Endpoint::Tcp { host, .. } => host.clone(),
            Endpoint::Socket { .. } => "localhost".to_owned(),
        };
        let mut make = tls.connector.clone();
        let connect =
            <tokio_postgres_rustls::MakeRustlsConnect as MakeTlsConnect<S>>::make_tls_connect(
                &mut make, &host_name,
            )
            .map_err(|error| Error::TlsFailed {
                target: target_name.to_owned(),
                detail: error.to_string(),
                source: None,
            })?;
        let (client, connection) =
            config
                .connect_raw(stream, connect)
                .await
                .map_err(|error| Error::ConnectFailed {
                    tried: vec![format!("{target_name} as {}", candidate.user)],
                    source: Box::new(error),
                })?;
        let driver = tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "the database connection ended");
            }
        });
        self.finish(
            client,
            driver,
            &tls,
            target_name.to_owned(),
            via,
            None,
            candidate.user.clone(),
        )
        .await
        .map_err(|error| Error::ConnectFailed {
            tried: vec![format!("{target_name} as {}", candidate.user)],
            source: error,
        })
    }

    async fn finish(
        &self,
        client: Client,
        driver: tokio::task::JoinHandle<()>,
        tls: &Arc<Tls>,
        target: String,
        via: Via,
        endpoint: Option<Endpoint>,
        user: String,
    ) -> std::result::Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        let pooled = self.settings.connection.pooled.value == Some(true);
        let schema = self.settings.schema.value.clone();
        let search_path = if pooled {
            String::new()
        } else {
            client
                .query_typed(
                    "SELECT pg_catalog.set_config('search_path', $1, false)",
                    &[(&schema, tokio_postgres::types::Type::TEXT)],
                )
                .await?;
            schema
        };
        let row = client
            .query_typed_one(
                "SELECT current_setting('server_version_num')::int4, current_setting('server_version'), current_user::text, current_database()::text",
                &[],
            )
            .await?;
        let server_version_num: i32 = row.try_get(0)?;
        let server_version: String = row.try_get(1)?;
        let role: String = row.try_get(2)?;
        let database: String = row.try_get(3)?;
        if !pooled && server_version_num >= 170_000 {
            let millis = self.settings.limits.transaction_timeout.value.as_millis();
            client
                .batch_execute(&format!("SET transaction_timeout = {millis}"))
                .await?;
        }
        let tls_used = match via {
            Via::Socket => Some(false),
            Via::Tcp | Via::Ssh => match tls.plan.driver_mode {
                tokio_postgres::config::SslMode::Disable => Some(false),
                tokio_postgres::config::SslMode::Require => Some(true),
                _ => None,
            },
        };
        let cancel = client.cancel_token();
        Ok(Session {
            client,
            cancel,
            info: SessionInfo {
                target,
                via,
                endpoint,
                user,
                tls: (via != Via::Socket).then_some(tls.plan),
                tls_used,
                server_version,
                server_version_num,
                role,
                database,
                search_path,
                pooled,
                attempts: Vec::new(),
            },
            tls: Arc::clone(tls),
            driver,
            keep: Vec::new(),
        })
    }
}

#[must_use]
pub fn session_settings(
    settings: &Settings,
    server_version_num: Option<i32>,
) -> Vec<(&'static str, String)> {
    let limits = &settings.limits;
    let mut out = vec![
        (
            "statement_timeout",
            limits.statement_timeout.value.as_millis().to_string(),
        ),
        (
            "lock_timeout",
            limits.lock_timeout.value.as_millis().to_string(),
        ),
        (
            "idle_in_transaction_session_timeout",
            (limits.handle_expiry.value + Duration::from_secs(5))
                .as_millis()
                .to_string(),
        ),
        ("client_encoding", "UTF8".to_owned()),
    ];
    if server_version_num.is_some_and(|version| version >= 170_000) {
        out.push((
            "transaction_timeout",
            limits.transaction_timeout.value.as_millis().to_string(),
        ));
    }
    out
}

#[must_use]
pub fn pooled_transaction_prefix(settings: &Settings, server_version_num: i32) -> Option<String> {
    if settings.connection.pooled.value != Some(true) {
        return None;
    }
    let mut statements = vec!["SET LOCAL search_path = ''".to_owned()];
    statements.extend(
        session_settings(settings, Some(server_version_num))
            .into_iter()
            .map(|(name, value)| {
                format!(
                    "SET LOCAL {name} = {}",
                    crate::render::quote_literal(&value)
                )
            }),
    );
    Some(statements.join("; "))
}

impl SessionInfo {
    #[must_use]
    pub fn tls_warning(&self) -> Option<String> {
        let plan = self.tls?;
        match (plan.driver_mode, plan.verification) {
            (tokio_postgres::config::SslMode::Disable, _) => Some(format!(
                "the connection to {} is not encrypted (sslmode=disable)",
                self.target
            )),
            (_, Verification::None) => Some(format!(
                "the connection to {} is not verified; set sslmode=verify-full with a root certificate for a remote server",
                self.target
            )),
            _ => None,
        }
    }
}

pub fn describe_sqlstate(error: &tokio_postgres::Error) -> Error {
    match error.as_db_error() {
        Some(db) => Error::SqlFailed {
            sqlstate: Some(db.code().code().to_owned()),
            message: db.message().to_owned(),
        },
        None => Error::SqlFailed {
            sqlstate: None,
            message: error.to_string(),
        },
    }
}

pub fn is_socket(host: Option<&str>) -> bool {
    host.is_none_or(presets::is_socket_directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Environment, FlagLayer, Sources, resolve};

    fn settings_for(flags: FlagLayer, env: &Environment) -> Settings {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::config::AppPaths::from_base(
            dir.path().join("c"),
            dir.path().join("d"),
            dir.path().join("k"),
        );
        resolve(
            flags,
            Sources {
                env,
                paths,
                keychain: None,
            },
        )
        .unwrap()
        .0
    }

    #[test]
    fn with_no_host_the_candidates_probe_sockets_then_loopback_as_postgres() {
        let env = Environment::default().with_os_user("sharkar");
        let settings = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                ..FlagLayer::default()
            },
            &env,
        );
        let connector = Connector::new(Arc::new(settings));
        let candidates = connector.candidates();
        let sockets = presets::socket_directories().len();
        assert_eq!(candidates.len(), sockets + 1);
        for candidate in &candidates[..sockets] {
            assert!(candidate.endpoint.is_socket());
            assert_eq!(candidate.user, "sharkar");
        }
        let last = candidates.last().unwrap();
        assert_eq!(
            last.endpoint,
            Endpoint::Tcp {
                host: "127.0.0.1".to_owned(),
                port: 5432
            }
        );
        assert_eq!(last.user, "postgres");
    }

    #[test]
    fn an_explicit_user_survives_into_the_loopback_fallback() {
        let env = Environment::default().with_os_user("sharkar");
        let settings = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                user: Some("claude_dev".to_owned()),
                ..FlagLayer::default()
            },
            &env,
        );
        let candidates = Connector::new(Arc::new(settings)).candidates();
        assert_eq!(candidates.last().unwrap().user, "claude_dev");
    }

    #[test]
    fn a_named_host_is_the_only_candidate_and_a_slash_means_a_socket() {
        let env = Environment::default();
        let tcp = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                host: Some("db.internal".to_owned()),
                port: Some(5433),
                ..FlagLayer::default()
            },
            &env,
        );
        let candidates = Connector::new(Arc::new(tcp)).candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].endpoint.to_string(), "db.internal:5433");
        let socket = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                host: Some("/tmp".to_owned()),
                ..FlagLayer::default()
            },
            &env,
        );
        let candidates = Connector::new(Arc::new(socket)).candidates();
        assert_eq!(candidates[0].endpoint.to_string(), "/tmp/.s.PGSQL.5432");
    }

    #[test]
    fn the_driver_config_carries_the_timeouts_and_the_encoding_unless_pooled() {
        let env = Environment::default().with_var("OWNPG_STATEMENT_TIMEOUT", "7");
        let settings = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                host: Some("db.internal".to_owned()),
                ..FlagLayer::default()
            },
            &env,
        );
        let connector = Connector::new(Arc::new(settings));
        let candidate = connector.candidates().remove(0);
        let config = connector.driver_config(&candidate, SslMode::Prefer);
        let options = config.get_options().unwrap();
        assert!(options.contains("statement_timeout=7000"), "{options}");
        assert!(
            options.contains("idle_in_transaction_session_timeout=65000"),
            "{options}"
        );
        assert!(options.contains("client_encoding=UTF8"), "{options}");
        assert_eq!(config.get_dbname(), Some("app"));
        assert!(config.get_application_name().unwrap().starts_with("ownpg/"));
        let pooled_env = Environment::default().with_var("OWNPG_POOLED", "true");
        let pooled = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                host: Some("db.internal".to_owned()),
                ..FlagLayer::default()
            },
            &pooled_env,
        );
        let connector = Connector::new(Arc::new(pooled));
        let candidate = connector.candidates().remove(0);
        assert!(
            connector
                .driver_config(&candidate, SslMode::Prefer)
                .get_options()
                .is_none()
        );
        let prefix = pooled_transaction_prefix(&connector.settings, 180_000).unwrap();
        assert!(
            prefix.starts_with("SET LOCAL search_path = ''; "),
            "{prefix}"
        );
        assert!(
            prefix.contains("SET LOCAL statement_timeout = '30000'"),
            "{prefix}"
        );
        assert!(
            prefix.contains("SET LOCAL client_encoding = 'UTF8'"),
            "{prefix}"
        );
        assert!(
            prefix.contains("SET LOCAL transaction_timeout = '"),
            "{prefix}"
        );
        assert!(
            !pooled_transaction_prefix(&connector.settings, 160_000)
                .unwrap()
                .contains("transaction_timeout"),
            "transaction_timeout exists from 17 on"
        );
        let plain = settings_for(
            FlagLayer {
                database: Some("app".to_owned()),
                ..FlagLayer::default()
            },
            &env,
        );
        assert!(pooled_transaction_prefix(&plain, 180_000).is_none());
    }

    #[test]
    fn the_tls_warning_fires_for_unverified_and_unencrypted_tcp_only() {
        let info = |via: Via, mode: tokio_postgres::config::SslMode, verification: Verification| {
            SessionInfo {
                target: "db:5432".to_owned(),
                via,
                endpoint: None,
                user: String::new(),
                tls: (via != Via::Socket).then_some(TlsPlan {
                    driver_mode: mode,
                    verification,
                    roots: tls::RootSource::NotNeeded,
                    client_certificate: false,
                }),
                tls_used: None,
                server_version: String::new(),
                server_version_num: 180_000,
                role: String::new(),
                database: String::new(),
                search_path: String::new(),
                pooled: false,
                attempts: Vec::new(),
            }
        };
        assert!(
            info(
                Via::Socket,
                tokio_postgres::config::SslMode::Disable,
                Verification::None
            )
            .tls_warning()
            .is_none()
        );
        assert!(
            info(
                Via::Tcp,
                tokio_postgres::config::SslMode::Disable,
                Verification::None
            )
            .tls_warning()
            .unwrap()
            .contains("not encrypted")
        );
        assert!(
            info(
                Via::Tcp,
                tokio_postgres::config::SslMode::Require,
                Verification::None
            )
            .tls_warning()
            .unwrap()
            .contains("not verified")
        );
        assert!(
            info(
                Via::Tcp,
                tokio_postgres::config::SslMode::Require,
                Verification::Full
            )
            .tls_warning()
            .is_none()
        );
    }
}
