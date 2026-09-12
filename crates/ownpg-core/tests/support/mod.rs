#![allow(
    dead_code,
    reason = "each integration test binary uses the subset of helpers it needs"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ownpg_core::config::{AppPaths, Environment, FlagLayer, Settings, Sources, resolve};
use tokio_postgres::NoTls;

pub(crate) const DSN_VARIABLE: &str = "OWNPG_TEST_DSN";

pub(crate) struct Scratch {
    pub(crate) maintenance_dsn: String,
    pub(crate) database: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) user: String,
    _dir: tempfile::TempDir,
    pub(crate) paths: AppPaths,
}

pub(crate) fn maintenance_dsn() -> Option<String> {
    std::env::var(DSN_VARIABLE)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(crate) async fn scratch() -> Option<Scratch> {
    let Some(maintenance_dsn) = maintenance_dsn() else {
        eprintln!("{DSN_VARIABLE} is not set; the live database tests are skipped");
        return None;
    };
    let config: tokio_postgres::Config = maintenance_dsn
        .parse()
        .expect("OWNPG_TEST_DSN is a libpq connection string");
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        #[cfg(unix)]
        Some(tokio_postgres::config::Host::Unix(path)) => path.display().to_string(),
        None => "127.0.0.1".to_owned(),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    let user = config.get_user().unwrap_or("postgres").to_owned();
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let database = format!("ownpg_test_{}_{sequence}", std::process::id());
    let (client, connection) = config
        .connect(NoTls)
        .await
        .expect("the maintenance database answers");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let statement = format!(
        "CREATE DATABASE {database} WITH OWNER = {user} TEMPLATE = template0 ENCODING = 'UTF8' LOCALE_PROVIDER = 'libc' LOCALE = 'en_US.UTF-8' CONNECTION LIMIT = -1"
    );
    client
        .batch_execute(&statement)
        .await
        .expect("the scratch database is created");
    let dir = tempfile::tempdir().expect("a temporary home for the test");
    let paths = AppPaths::from_base(
        dir.path().join("config"),
        dir.path().join("data"),
        dir.path().join("cache"),
    );
    Some(Scratch {
        maintenance_dsn,
        database,
        host,
        port,
        user,
        _dir: dir,
        paths,
    })
}

impl Scratch {
    pub(crate) fn environment(&self) -> Environment {
        let mut vars = BTreeMap::new();
        vars.insert("OWNPG_DATABASE".to_owned(), self.database.clone());
        vars.insert("OWNPG_HOST".to_owned(), self.host.clone());
        vars.insert("OWNPG_PORT".to_owned(), self.port.to_string());
        vars.insert("OWNPG_USER".to_owned(), self.user.clone());
        Environment::new(vars, None, Some(self.user.clone()))
    }

    pub(crate) fn settings(&self, flags: FlagLayer) -> Settings {
        let env = self.environment();
        resolve(
            flags,
            Sources {
                env: &env,
                paths: self.paths.clone(),
                keychain: None,
            },
        )
        .expect("the scratch settings resolve")
        .0
    }

    pub(crate) fn settings_with(&self, flags: FlagLayer, extra: &[(&str, &str)]) -> Settings {
        let mut env = self.environment();
        for (name, value) in extra {
            env = env.with_var(name, value);
        }
        resolve(
            flags,
            Sources {
                env: &env,
                paths: self.paths.clone(),
                keychain: None,
            },
        )
        .expect("the scratch settings resolve")
        .0
    }

    pub(crate) async fn client(&self) -> tokio_postgres::Client {
        let mut config: tokio_postgres::Config = self
            .maintenance_dsn
            .parse()
            .expect("OWNPG_TEST_DSN is a libpq connection string");
        config.dbname(&self.database);
        let (client, connection) = config
            .connect(NoTls)
            .await
            .expect("the scratch database answers");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let dsn = self.maintenance_dsn.clone();
        let database = self.database.clone();
        let cleanup = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the scratch cleanup");
            runtime.block_on(async move {
                let config: tokio_postgres::Config =
                    dsn.parse().expect("a libpq connection string");
                let Ok((client, connection)) = config.connect(NoTls).await else {
                    return;
                };
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                let _ = client
                    .batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"))
                    .await;
            });
        });
        let _ = cleanup.join();
    }
}

pub(crate) fn shared(settings: Settings) -> Arc<Settings> {
    Arc::new(settings)
}
