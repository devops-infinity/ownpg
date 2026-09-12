#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    reason = "clippy.toml exempts test modules, and an integration test is a separate crate it cannot reach"
)]

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use ownpg_core::ErrorId;
use ownpg_core::config::profile::{ProfileEntry, ProfileFile, SshEntry};
use ownpg_core::config::{Environment, FlagLayer, Sources, resolve};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::connect::{Connector, Via};
use russh::keys::known_hosts::learn_known_hosts_path;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;

struct Bastion {
    accepted_key: PublicKey,
    user: String,
}

impl server::Handler for Bastion {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if user == self.user && *public_key == self.accepted_key {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let target = format!("{host_to_connect}:{port_to_connect}");
        match tokio::net::TcpStream::connect(&target).await {
            Ok(mut upstream) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
                });
            }
            Err(_) => {
                reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct RunningBastion {
    port: u16,
    host_key: PublicKey,
    _task: tokio::task::JoinHandle<()>,
}

async fn start_bastion(client_key: PublicKey) -> RunningBastion {
    let host_key = PrivateKey::random(&mut rand::rng(), ssh_key::Algorithm::Ed25519).unwrap();
    let public = host_key.public_key().clone();
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::from_millis(10),
        ..server::Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let handler = Bastion {
                accepted_key: client_key.clone(),
                user: "deploy".to_owned(),
            };
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, stream, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    RunningBastion {
        port,
        host_key: public,
        _task: task,
    }
}

struct ClientKey {
    path: PathBuf,
    public: PublicKey,
}

fn write_client_key(dir: &std::path::Path) -> ClientKey {
    let key = PrivateKey::random(&mut rand::rng(), ssh_key::Algorithm::Ed25519).unwrap();
    let path = dir.join("id_ed25519");
    let encoded = key.to_openssh(ssh_key::LineEnding::LF).unwrap();
    std::fs::write(&path, encoded.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    ClientKey {
        path,
        public: key.public_key().clone(),
    }
}

fn settings_for(
    scratch: &support::Scratch,
    ssh: SshEntry,
    trust_new_host: Option<bool>,
) -> ownpg_core::config::Settings {
    let mut file = ProfileFile::default();
    file.profiles.insert(
        "tunnel".to_owned(),
        ProfileEntry {
            database: Some(scratch.database.clone()),
            host: Some(scratch.host.clone()),
            port: Some(scratch.port),
            user: Some(scratch.user.clone()),
            ssh: Some(ssh),
            ..ProfileEntry::default()
        },
    );
    file.save(&scratch.paths.config_file).unwrap();
    let env = Environment::new(BTreeMap::new(), None, Some("tester".to_owned()));
    resolve(
        FlagLayer {
            profile: Some("tunnel".to_owned()),
            ssh_trust_new_host: trust_new_host,
            ..FlagLayer::default()
        },
        Sources {
            env: &env,
            paths: scratch.paths.clone(),
            keychain: None,
        },
    )
    .unwrap()
    .0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tunnel_through_an_in_process_bastion_reaches_postgresql() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    if scratch.host.starts_with('/') {
        eprintln!("the maintenance DSN uses a socket; the tunnel test needs a TCP host");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let client_key = write_client_key(dir.path());
    let bastion = start_bastion(client_key.public.clone()).await;
    let known_hosts = dir.path().join("known_hosts");
    learn_known_hosts_path("127.0.0.1", bastion.port, &bastion.host_key, &known_hosts).unwrap();
    let settings = settings_for(
        &scratch,
        SshEntry {
            host: "127.0.0.1".to_owned(),
            port: Some(bastion.port),
            user: Some("deploy".to_owned()),
            key_file: Some(client_key.path.clone()),
            agent: Some(false),
            known_hosts: Some(known_hosts),
            ..SshEntry::default()
        },
        None,
    );
    let connector = Connector::new(Arc::new(settings)).with_ssh_hints(Hints::default());
    let session = connector.connect().await.expect("the tunnel connects");
    assert_eq!(session.info.via, Via::Ssh);
    assert!(
        session.info.target.contains("via deploy@127.0.0.1"),
        "{}",
        session.info.target
    );
    let one: i32 = session
        .client
        .query_one("SELECT 1", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(one, 1);
    assert_eq!(session.info.database, scratch.database);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_host_key_is_refused_with_its_fingerprint_unless_trusted() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    if scratch.host.starts_with('/') {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let client_key = write_client_key(dir.path());
    let bastion = start_bastion(client_key.public.clone()).await;
    let known_hosts = dir.path().join("known_hosts");
    let entry = SshEntry {
        host: "127.0.0.1".to_owned(),
        port: Some(bastion.port),
        user: Some("deploy".to_owned()),
        key_file: Some(client_key.path.clone()),
        agent: Some(false),
        known_hosts: Some(known_hosts.clone()),
        ..SshEntry::default()
    };
    let strict = Connector::new(Arc::new(settings_for(&scratch, entry.clone(), None)));
    let error = strict
        .connect()
        .await
        .expect_err("an unknown key is refused");
    assert_eq!(error.id(), ErrorId::SshHostKeyUnknown);
    assert!(error.to_string().contains("SHA256:"), "{error}");
    assert!(!known_hosts.exists());

    let trusting = Connector::new(Arc::new(settings_for(&scratch, entry, Some(true))));
    let session = trusting
        .connect()
        .await
        .expect("trust-on-first-use connects");
    assert_eq!(session.info.via, Via::Ssh);
    let recorded = std::fs::read_to_string(&known_hosts).unwrap();
    assert!(
        recorded.contains(&format!("[127.0.0.1]:{}", bastion.port)),
        "{recorded}"
    );
    drop(session);

    let again = Connector::new(Arc::new(settings_for(
        &scratch,
        SshEntry {
            host: "127.0.0.1".to_owned(),
            port: Some(bastion.port),
            user: Some("deploy".to_owned()),
            key_file: Some(client_key.path),
            agent: Some(false),
            known_hosts: Some(known_hosts),
            ..SshEntry::default()
        },
        None,
    )));
    again.connect().await.expect("the learned key is now known");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_jump_host_chain_reaches_the_bastion_and_then_postgresql() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    if scratch.host.starts_with('/') {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let client_key = write_client_key(dir.path());
    let first = start_bastion(client_key.public.clone()).await;
    let second = start_bastion(client_key.public.clone()).await;
    let known_hosts = dir.path().join("known_hosts");
    learn_known_hosts_path("127.0.0.1", first.port, &first.host_key, &known_hosts).unwrap();
    learn_known_hosts_path("127.0.0.1", second.port, &second.host_key, &known_hosts).unwrap();
    let settings = settings_for(
        &scratch,
        SshEntry {
            host: "127.0.0.1".to_owned(),
            port: Some(second.port),
            user: Some("deploy".to_owned()),
            key_file: Some(client_key.path),
            agent: Some(false),
            known_hosts: Some(known_hosts),
            jump: Some(vec![format!("deploy@127.0.0.1:{}", first.port)]),
            ..SshEntry::default()
        },
        None,
    );
    let session = Connector::new(Arc::new(settings))
        .connect()
        .await
        .expect("the two-hop tunnel connects");
    assert_eq!(session.info.via, Via::Ssh);
    assert!(
        session.info.target.contains(" -> "),
        "{}",
        session.info.target
    );
    let one: i32 = session
        .client
        .query_one("SELECT 1", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(one, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_key_is_refused_with_every_method_named() {
    let Some(scratch) = support::scratch().await else {
        return;
    };
    if scratch.host.starts_with('/') {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let accepted = write_client_key(dir.path());
    let other_dir = tempfile::tempdir().unwrap();
    let rejected = write_client_key(other_dir.path());
    let bastion = start_bastion(accepted.public.clone()).await;
    let known_hosts = dir.path().join("known_hosts");
    learn_known_hosts_path("127.0.0.1", bastion.port, &bastion.host_key, &known_hosts).unwrap();
    let settings = settings_for(
        &scratch,
        SshEntry {
            host: "127.0.0.1".to_owned(),
            port: Some(bastion.port),
            user: Some("deploy".to_owned()),
            key_file: Some(rejected.path),
            agent: Some(false),
            known_hosts: Some(known_hosts),
            ..SshEntry::default()
        },
        None,
    );
    let error = Connector::new(Arc::new(settings))
        .connect()
        .await
        .expect_err("the wrong key is refused");
    assert_eq!(error.id(), ErrorId::SshFailed);
    assert!(error.to_string().contains("refused for deploy"), "{error}");
}
