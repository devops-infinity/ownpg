use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, AuthResult, Handle, Handler};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key};

use crate::config::{SshSettings, SshTransport, parse_ssh_target};
use crate::error::{Error, Result};

use super::BoxedStream;

const DEFAULT_KEY_FILES: [&str; 3] = [".ssh/id_ed25519", ".ssh/id_ecdsa", ".ssh/id_rsa"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct Hints {
    pub home: Option<PathBuf>,
    pub agent_socket: Option<PathBuf>,
    pub os_user: Option<String>,
}

pub struct Tunnel {
    pub stream: BoxedStream,
    pub route: Vec<String>,
    keep: Vec<Box<dyn std::any::Any + Send>>,
}

impl std::fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tunnel")
            .field("route", &self.route)
            .finish()
    }
}

impl Tunnel {
    pub fn into_parts(self) -> (BoxedStream, Vec<String>, Vec<Box<dyn std::any::Any + Send>>) {
        (self.stream, self.route, self.keep)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostKeyVerdict {
    Known,
    Learned(String),
    Unknown(String),
    Changed(usize),
    Certificate,
}

#[derive(Debug, Clone)]
struct HostKeyHandler {
    host: String,
    port: u16,
    known_hosts: Option<PathBuf>,
    trust_new_host: bool,
    verdict: Arc<Mutex<Option<HostKeyVerdict>>>,
}

impl HostKeyHandler {
    fn record(&self, verdict: HostKeyVerdict) -> bool {
        let accepted = matches!(verdict, HostKeyVerdict::Known | HostKeyVerdict::Learned(_));
        if let Ok(mut slot) = self.verdict.lock() {
            *slot = Some(verdict);
        }
        accepted
    }
}

impl Handler for HostKeyHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Ok(self.record(HostKeyVerdict::Certificate));
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let Some(path) = &self.known_hosts else {
            return Ok(self.record(HostKeyVerdict::Unknown(fingerprint)));
        };
        match check_known_hosts_path(&self.host, self.port, key, path) {
            Ok(true) => Ok(self.record(HostKeyVerdict::Known)),
            Ok(false) => {
                if self.trust_new_host {
                    if let Err(error) = learn_known_hosts_path(&self.host, self.port, key, path) {
                        tracing::warn!(%error, "the host key could not be recorded");
                    }
                    Ok(self.record(HostKeyVerdict::Learned(fingerprint)))
                } else {
                    Ok(self.record(HostKeyVerdict::Unknown(fingerprint)))
                }
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                Ok(self.record(HostKeyVerdict::Changed(line)))
            }
            Err(error) => {
                tracing::warn!(%error, "the known hosts file could not be read");
                Ok(self.record(HostKeyVerdict::Unknown(fingerprint)))
            }
        }
    }
}

fn ssh_error(host: &str, detail: impl Into<String>) -> Error {
    Error::SshFailed {
        host: host.to_owned(),
        detail: detail.into(),
        source: None,
    }
}

pub fn plan_route(settings: &SshSettings, hints: &Hints) -> Result<Vec<Hop>> {
    let mut hops = Vec::new();
    let mut specs: Vec<(Option<String>, String, Option<u16>)> = Vec::new();
    for jump in &settings.jump.value {
        let target = parse_ssh_target(jump)?;
        specs.push((target.user, target.host, target.port));
    }
    specs.push((
        settings.user.as_ref().map(|user| user.value.clone()),
        settings.host.value.clone(),
        Some(settings.port.value),
    ));
    let alias_config = settings.config_file.as_ref().map(|file| file.value.clone());
    let mut extra_jumps: Vec<(Option<String>, String, Option<u16>)> = Vec::new();
    for (index, (user, host, port)) in specs.iter().enumerate() {
        let mut hop = Hop {
            host: host.clone(),
            port: port.unwrap_or(22),
            user: user
                .clone()
                .or_else(|| hints.os_user.clone())
                .unwrap_or_else(|| "root".to_owned()),
            key_files: Vec::new(),
        };
        if let Some(config) = &alias_config
            && config.is_file()
            && let Ok(resolved) = russh_config::parse_path(config, host)
        {
            if let Some(real_host) = resolved.host_config.hostname.clone() {
                hop.host = real_host;
            }
            if user.is_none()
                && let Some(alias_user) = resolved.host_config.user.clone()
            {
                hop.user = alias_user;
            }
            if port.is_none()
                && let Some(alias_port) = resolved.host_config.port
            {
                hop.port = alias_port;
            }
            if let Some(files) = &resolved.host_config.identity_file {
                hop.key_files.extend(files.iter().cloned());
            }
            if index + 1 == specs.len()
                && let Some(proxy) = &resolved.host_config.proxy_jump
            {
                for jump in proxy
                    .split(',')
                    .map(str::trim)
                    .filter(|jump| !jump.is_empty())
                {
                    let target = parse_ssh_target(jump)?;
                    extra_jumps.push((target.user, target.host, target.port));
                }
            }
        }
        if let Some(key) = &settings.key_file {
            hop.key_files.push(key.value.clone());
        }
        if hop.key_files.is_empty()
            && let Some(home) = &hints.home
        {
            for relative in DEFAULT_KEY_FILES {
                let candidate = home.join(relative);
                if candidate.is_file() {
                    hop.key_files.push(candidate);
                }
            }
        }
        hops.push(hop);
    }
    if !extra_jumps.is_empty() {
        let last = hops
            .pop()
            .ok_or_else(|| ssh_error(&settings.host.value, "no hop"))?;
        for (user, host, port) in extra_jumps {
            hops.push(Hop {
                host,
                port: port.unwrap_or(22),
                user: user
                    .or_else(|| hints.os_user.clone())
                    .unwrap_or_else(|| last.user.clone()),
                key_files: last.key_files.clone(),
            });
        }
        hops.push(last);
    }
    Ok(hops)
}

pub async fn open(
    settings: &SshSettings,
    target_host: &str,
    target_port: u16,
    hints: &Hints,
) -> Result<Tunnel> {
    match settings.transport.value {
        SshTransport::InProcess => open_in_process(settings, target_host, target_port, hints).await,
        SshTransport::System => open_system(settings, target_host, target_port, hints).await,
    }
}

async fn open_in_process(
    settings: &SshSettings,
    target_host: &str,
    target_port: u16,
    hints: &Hints,
) -> Result<Tunnel> {
    let hops = plan_route(settings, hints)?;
    let timeout = settings.connect_timeout.value;
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..client::Config::default()
    });
    let mut route = Vec::new();
    let mut handles: Vec<Handle<HostKeyHandler>> = Vec::new();
    for hop in &hops {
        let verdict = Arc::new(Mutex::new(None));
        let handler = HostKeyHandler {
            host: hop.host.clone(),
            port: hop.port,
            known_hosts: settings.known_hosts.as_ref().map(|path| path.value.clone()),
            trust_new_host: settings.trust_new_host.value,
            verdict: Arc::clone(&verdict),
        };
        let connected = match handles.last() {
            None => {
                tokio::time::timeout(
                    timeout,
                    client::connect(Arc::clone(&config), (hop.host.as_str(), hop.port), handler),
                )
                .await
            }
            Some(previous) => {
                let channel = tokio::time::timeout(
                    timeout,
                    previous.channel_open_direct_tcpip(
                        hop.host.as_str(),
                        u32::from(hop.port),
                        "127.0.0.1",
                        0,
                    ),
                )
                .await
                .map_err(|_| ssh_error(&hop.host, "the jump host did not answer in time"))?
                .map_err(|error| {
                    ssh_error(&hop.host, format!("the jump channel failed: {error}"))
                })?;
                tokio::time::timeout(
                    timeout,
                    client::connect_stream(Arc::clone(&config), channel.into_stream(), handler),
                )
                .await
            }
        };
        let mut handle = match connected {
            Ok(Ok(handle)) => handle,
            Ok(Err(error)) => {
                let recorded = verdict.lock().ok().and_then(|slot| slot.clone());
                return Err(host_key_error(&hop.host, hop.port, recorded, error));
            }
            Err(_) => {
                return Err(ssh_error(
                    &hop.host,
                    format!("no answer within {} seconds", timeout.as_secs()),
                ));
            }
        };
        authenticate(&mut handle, hop, settings, hints, timeout).await?;
        route.push(format!("{}@{}:{}", hop.user, hop.host, hop.port));
        handles.push(handle);
    }
    let last = handles
        .last()
        .ok_or_else(|| ssh_error(&settings.host.value, "no hop"))?;
    let channel = tokio::time::timeout(
        timeout,
        last.channel_open_direct_tcpip(target_host, u32::from(target_port), "127.0.0.1", 0),
    )
    .await
    .map_err(|_| {
        ssh_error(
            &settings.host.value,
            "the forwarded channel did not open in time",
        )
    })?
    .map_err(|error| {
        ssh_error(
            &settings.host.value,
            format!("the bastion could not reach {target_host}:{target_port}: {error}"),
        )
    })?;
    let stream: BoxedStream = Box::new(channel.into_stream());
    Ok(Tunnel {
        stream,
        route,
        keep: vec![Box::new(handles)],
    })
}

fn host_key_error(
    host: &str,
    port: u16,
    verdict: Option<HostKeyVerdict>,
    error: russh::Error,
) -> Error {
    match verdict {
        Some(HostKeyVerdict::Unknown(fingerprint)) => Error::SshHostKeyUnknown {
            host: format!("{host}:{port}"),
            fingerprint,
        },
        Some(HostKeyVerdict::Changed(line)) => ssh_error(
            host,
            format!(
                "the host key does not match the one recorded at line {line} of the known hosts file; this can be a changed server or an attack, so nothing was sent"
            ),
        ),
        Some(HostKeyVerdict::Certificate) => ssh_error(
            host,
            "the server presented a host certificate, which this build does not check; record the host key with ssh first",
        ),
        _ => ssh_error(host, error.to_string()),
    }
}

async fn authenticate(
    handle: &mut Handle<HostKeyHandler>,
    hop: &Hop,
    settings: &SshSettings,
    hints: &Hints,
    timeout: Duration,
) -> Result<()> {
    let hash_alg = tokio::time::timeout(timeout, handle.best_supported_rsa_hash())
        .await
        .map_err(|_| ssh_error(&hop.host, "no answer while negotiating authentication"))?
        .map_err(|error| ssh_error(&hop.host, format!("authentication setup failed: {error}")))?
        .flatten();
    let mut tried = Vec::new();

    if settings.agent.value
        && let Some(socket) = &hints.agent_socket
    {
        match AgentClient::connect_uds(socket).await {
            Ok(mut agent) => match agent.request_identities().await {
                Ok(identities) => {
                    for identity in identities {
                        let AgentIdentity::PublicKey { key, comment } = identity else {
                            continue;
                        };
                        let outcome = tokio::time::timeout(
                            timeout,
                            handle.authenticate_publickey_with(
                                hop.user.clone(),
                                key,
                                hash_alg,
                                &mut agent,
                            ),
                        )
                        .await;
                        match outcome {
                            Ok(Ok(AuthResult::Success)) => return Ok(()),
                            Ok(Ok(AuthResult::Failure { .. })) => {
                                tried.push(format!("agent key {comment}"));
                            }
                            Ok(Err(error)) => {
                                tried.push(format!("agent key {comment} ({error})"));
                            }
                            Err(_) => {
                                return Err(ssh_error(
                                    &hop.host,
                                    "no answer during agent authentication",
                                ));
                            }
                        }
                    }
                }
                Err(error) => tried.push(format!("agent ({error})")),
            },
            Err(error) => tried.push(format!("agent socket ({error})")),
        }
    }

    for key_file in &hop.key_files {
        let passphrase = settings
            .password
            .as_ref()
            .map(|value| value.value.expose().to_owned());
        let loaded = match load_secret_key(key_file, None) {
            Ok(key) => Ok(key),
            Err(first) => match passphrase.as_deref() {
                Some(phrase) => load_secret_key(key_file, Some(phrase)).map_err(|_| first),
                None => Err(first),
            },
        };
        let key = match loaded {
            Ok(key) => key,
            Err(error) => {
                tried.push(format!("{} ({error})", key_file.display()));
                continue;
            }
        };
        let outcome = tokio::time::timeout(
            timeout,
            handle.authenticate_publickey(
                hop.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
            ),
        )
        .await;
        match outcome {
            Ok(Ok(AuthResult::Success)) => return Ok(()),
            Ok(Ok(AuthResult::Failure { .. })) => tried.push(key_file.display().to_string()),
            Ok(Err(error)) => tried.push(format!("{} ({error})", key_file.display())),
            Err(_) => return Err(ssh_error(&hop.host, "no answer during key authentication")),
        }
    }

    if let Some(password) = &settings.password {
        let outcome = tokio::time::timeout(
            timeout,
            handle.authenticate_password(hop.user.clone(), password.value.expose().to_owned()),
        )
        .await;
        match outcome {
            Ok(Ok(AuthResult::Success)) => return Ok(()),
            Ok(Ok(AuthResult::Failure { .. })) => tried.push("password".to_owned()),
            Ok(Err(error)) => tried.push(format!("password ({error})")),
            Err(_) => {
                return Err(ssh_error(
                    &hop.host,
                    "no answer during password authentication",
                ));
            }
        }
    }

    Err(ssh_error(
        &hop.host,
        if tried.is_empty() {
            format!("no authentication method is configured for {}", hop.user)
        } else {
            format!(
                "every authentication method was refused for {}: {}",
                hop.user,
                tried.join(", ")
            )
        },
    ))
}

#[cfg(unix)]
async fn open_system(
    settings: &SshSettings,
    target_host: &str,
    target_port: u16,
    hints: &Hints,
) -> Result<Tunnel> {
    use openssh::{ForwardType, KnownHosts, SessionBuilder, Socket};

    let hops = plan_route(settings, hints)?;
    let bastion = hops
        .last()
        .ok_or_else(|| ssh_error(&settings.host.value, "no hop"))?;
    let mut builder = SessionBuilder::default();
    builder
        .known_hosts_check(if settings.trust_new_host.value {
            KnownHosts::Add
        } else {
            KnownHosts::Strict
        })
        .connect_timeout(settings.connect_timeout.value)
        .user(bastion.user.clone())
        .port(bastion.port);
    if let Some(key) = &settings.key_file {
        builder.keyfile(&key.value);
    }
    if let Some(config) = &settings.config_file
        && config.value.is_file()
    {
        builder.config_file(&config.value);
    }
    let session = builder
        .connect(&bastion.host)
        .await
        .map_err(|error| ssh_error(&bastion.host, format!("the system ssh failed: {error}")))?;
    let socket_dir = tempfile::Builder::new()
        .prefix("ownpg-ssh-")
        .tempdir()
        .map_err(|error| {
            ssh_error(
                &bastion.host,
                format!("no directory for the forward socket: {error}"),
            )
        })?;
    let socket_path = socket_dir.path().join("forward.sock");
    session
        .request_port_forward(
            ForwardType::Local,
            Socket::UnixSocket {
                path: std::borrow::Cow::Borrowed(socket_path.as_path()),
            },
            Socket::TcpSocket {
                host: std::borrow::Cow::Borrowed(target_host),
                port: target_port,
            },
        )
        .await
        .map_err(|error| {
            ssh_error(
                &bastion.host,
                format!("the port forward was refused: {error}"),
            )
        })?;
    let stream = tokio::net::UnixStream::connect(&socket_path)
        .await
        .map_err(|error| {
            ssh_error(
                &bastion.host,
                format!("the forward socket did not answer: {error}"),
            )
        })?;
    Ok(Tunnel {
        stream: Box::new(stream),
        route: vec![format!(
            "{}@{}:{} (system ssh)",
            bastion.user, bastion.host, bastion.port
        )],
        keep: vec![Box::new(session), Box::new(socket_dir)],
    })
}

#[cfg(not(unix))]
async fn open_system(
    settings: &SshSettings,
    _target_host: &str,
    _target_port: u16,
    _hints: &Hints,
) -> Result<Tunnel> {
    Err(ssh_error(
        &settings.host.value,
        "the system ssh transport is available on macOS and Linux only; use the in-process transport",
    ))
}

pub fn agent_socket_from(value: Option<&str>) -> Option<PathBuf> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

pub fn known_hosts_default(home: Option<&Path>) -> Option<PathBuf> {
    home.map(|home| home.join(".ssh").join("known_hosts"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Origin, Resolved, Secret};

    fn settings(host: &str, jump: Vec<String>, config_file: Option<PathBuf>) -> SshSettings {
        SshSettings {
            host: Resolved::new(host.to_owned(), Origin::Flag),
            port: Resolved::preset(22),
            user: Some(Resolved::new("deploy".to_owned(), Origin::Flag)),
            key_file: None,
            agent: Resolved::preset(true),
            password: None,
            trust_new_host: Resolved::preset(false),
            transport: Resolved::preset(SshTransport::InProcess),
            jump: Resolved::preset(jump),
            known_hosts: None,
            config_file: config_file.map(|path| Resolved::new(path, Origin::Profile)),
            connect_timeout: Resolved::preset(Duration::from_secs(10)),
        }
    }

    #[test]
    fn the_route_lists_every_jump_before_the_bastion() {
        let hints = Hints {
            os_user: Some("me".to_owned()),
            ..Hints::default()
        };
        let hops = plan_route(
            &settings(
                "bastion",
                vec!["first".to_owned(), "ops@second:2200".to_owned()],
                None,
            ),
            &hints,
        )
        .unwrap();
        assert_eq!(hops.len(), 3);
        assert_eq!(hops[0].host, "first");
        assert_eq!(hops[0].user, "me");
        assert_eq!(hops[1].host, "second");
        assert_eq!(hops[1].port, 2200);
        assert_eq!(hops[1].user, "ops");
        assert_eq!(hops[2].host, "bastion");
        assert_eq!(hops[2].user, "deploy");
    }

    #[test]
    fn an_alias_in_the_ssh_config_supplies_the_host_name_the_user_and_the_jump() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(
            &config,
            "Host staging\n  HostName staging.internal\n  User ops\n  Port 2222\n  IdentityFile /keys/staging\n  ProxyJump edge@gate.example.com:2200\n",
        )
        .unwrap();
        let mut resolved = settings("staging", Vec::new(), Some(config));
        resolved.user = None;
        let hints = Hints::default();
        let hops = plan_route(&resolved, &hints).unwrap();
        assert_eq!(hops.len(), 2);
        assert_eq!(hops[0].host, "gate.example.com");
        assert_eq!(hops[0].port, 2200);
        assert_eq!(hops[0].user, "edge");
        assert_eq!(hops[1].host, "staging.internal");
        assert_eq!(hops[1].user, "ops");
        assert_eq!(hops[1].key_files, vec![PathBuf::from("/keys/staging")]);
    }

    #[test]
    fn default_key_files_are_the_ones_that_exist_under_home() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        std::fs::write(dir.path().join(".ssh/id_ed25519"), "x").unwrap();
        let hints = Hints {
            home: Some(dir.path().to_path_buf()),
            ..Hints::default()
        };
        let hops = plan_route(&settings("bastion", Vec::new(), None), &hints).unwrap();
        assert_eq!(hops[0].key_files, vec![dir.path().join(".ssh/id_ed25519")]);
    }

    #[test]
    fn an_unknown_host_key_becomes_the_refusal_with_its_fingerprint() {
        let error = host_key_error(
            "bastion",
            22,
            Some(HostKeyVerdict::Unknown("SHA256:abc".to_owned())),
            russh::Error::Disconnect,
        );
        assert_eq!(error.id(), crate::error::ErrorId::SshHostKeyUnknown);
        assert!(error.to_string().contains("SHA256:abc"));
        assert!(error.remedy().contains("--ssh-trust-new-host"));
        let changed = host_key_error(
            "bastion",
            22,
            Some(HostKeyVerdict::Changed(4)),
            russh::Error::Disconnect,
        );
        assert!(changed.to_string().contains("line 4"));
    }

    #[test]
    fn the_password_secret_stays_out_of_debug_output() {
        let mut with_password = settings("bastion", Vec::new(), None);
        with_password.password = Some(Resolved::new(
            Secret::new("hunter2".to_owned()),
            Origin::Profile,
        ));
        let shown = format!("{with_password:?}");
        assert!(!shown.contains("hunter2"));
    }
}
