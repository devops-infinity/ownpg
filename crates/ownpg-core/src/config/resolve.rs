use std::path::PathBuf;
use std::time::Duration;

use super::environment::Environment;
use super::libpq::{self, LibpqLayer, PasswordFileOutcome};
use super::profile::{ProfileEntry, ProfileFile};
use super::{
    AppPaths, AuditSettings, ChannelBinding, ConnectionSettings, DEFAULT_AUDIT_MAX_BYTES,
    DEFAULT_BYTE_CAP, DEFAULT_CONNECT_TIMEOUT, DEFAULT_HANDLE_EXPIRY, DEFAULT_LOCK_TIMEOUT,
    DEFAULT_PORT, DEFAULT_ROW_CAP, DEFAULT_SCHEMA, DEFAULT_STATEMENT_TIMEOUT,
    DEFAULT_TRANSACTION_TIMEOUT, LimitSettings, MAX_ROW_CAP, Mode, Origin, Resolved, Secret,
    Settings, SshSettings, SshTransport, SslMode, ToolGroup,
};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagLayer {
    pub profile: Option<String>,
    pub mode: Option<Mode>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub sslmode: Option<SslMode>,
    pub sslrootcert: Option<PathBuf>,
    pub tools: Option<Vec<ToolGroup>>,
    pub strict_role: Option<bool>,
    pub ssh: Option<String>,
    pub ssh_transport: Option<SshTransport>,
    pub ssh_trust_new_host: Option<bool>,
    pub no_input: Option<bool>,
    pub audit: Option<bool>,
    pub audit_path: Option<PathBuf>,
    pub pg_bindir: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
}

pub type KeychainLookup<'a> = &'a dyn Fn(&str) -> Result<Option<String>>;

pub struct Sources<'a> {
    pub env: &'a Environment,
    pub paths: AppPaths,
    pub keychain: Option<KeychainLookup<'a>>,
}

impl std::fmt::Debug for Sources<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sources")
            .field("env", self.env)
            .field("paths", &self.paths)
            .field("keychain", &self.keychain.is_some())
            .finish()
    }
}

struct Pick<T> {
    flag: Option<T>,
    env: Option<T>,
    profile: Option<T>,
    libpq: Option<T>,
}

impl<T> Pick<T> {
    const fn new(flag: Option<T>, env: Option<T>, profile: Option<T>) -> Self {
        Self {
            flag,
            env,
            profile,
            libpq: None,
        }
    }

    fn with_libpq(mut self, libpq: Option<T>) -> Self {
        self.libpq = libpq;
        self
    }

    fn resolve(self) -> Option<Resolved<T>> {
        if let Some(value) = self.flag {
            return Some(Resolved::new(value, Origin::Flag));
        }
        if let Some(value) = self.env {
            return Some(Resolved::new(value, Origin::Environment));
        }
        if let Some(value) = self.profile {
            return Some(Resolved::new(value, Origin::Profile));
        }
        self.libpq.map(|value| Resolved::new(value, Origin::Libpq))
    }

    fn or_preset(self, preset: T) -> Resolved<T> {
        self.resolve().unwrap_or_else(|| Resolved::preset(preset))
    }
}

fn env_bool(env: &Environment, name: &str) -> Result<Option<bool>> {
    let Some(raw) = env.var(name) else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        other => Err(Error::ConfigInvalid {
            setting: name.to_owned(),
            value: other.to_owned(),
            detail: "expected true or false".to_owned(),
        }),
    }
}

fn env_u64(env: &Environment, name: &str) -> Result<Option<u64>> {
    let Some(raw) = env.var(name) else {
        return Ok(None);
    };
    raw.trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|_| Error::ConfigInvalid {
            setting: name.to_owned(),
            value: raw.to_owned(),
            detail: "expected a whole number".to_owned(),
        })
}

fn env_parsed<T>(
    env: &Environment,
    name: &str,
    parse: fn(&str) -> Option<T>,
    accepted: &str,
) -> Result<Option<T>> {
    let Some(raw) = env.var(name) else {
        return Ok(None);
    };
    parse(raw).map(Some).ok_or_else(|| Error::ConfigInvalid {
        setting: name.to_owned(),
        value: raw.to_owned(),
        detail: format!("expected {accepted}"),
    })
}

fn env_path(env: &Environment, name: &str) -> Option<PathBuf> {
    env.var(name).map(PathBuf::from)
}

pub fn parse_tool_groups(text: &str) -> Result<Vec<ToolGroup>> {
    let mut groups = Vec::new();
    for item in text
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let group = ToolGroup::parse(item).ok_or_else(|| Error::ConfigInvalid {
            setting: "tools".to_owned(),
            value: item.to_owned(),
            detail: "expected write, transactions, ddl, roles, maintenance, monitoring, or host"
                .to_owned(),
        })?;
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    Ok(groups)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

pub fn parse_ssh_target(text: &str) -> Result<SshTarget> {
    let invalid = |detail: &str| Error::ConfigInvalid {
        setting: "ssh".to_owned(),
        value: text.to_owned(),
        detail: detail.to_owned(),
    };
    let text = text.trim();
    let (user, rest) = match text.rsplit_once('@') {
        Some((user, rest)) => (Some(user.to_owned()), rest),
        None => (None, text),
    };
    let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
        let (host, tail) = inner
            .split_once(']')
            .ok_or_else(|| invalid("an IPv6 host needs a closing bracket"))?;
        (host.to_owned(), tail.strip_prefix(':'))
    } else {
        match rest.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host.to_owned(), Some(port)),
            _ => (rest.to_owned(), None),
        }
    };
    if host.is_empty() {
        return Err(invalid("expected `user@host`, `host`, or `user@host:port`"));
    }
    let port = match port {
        Some(port) => Some(
            port.parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| invalid("the port must be a number from 1 to 65535"))?,
        ),
        None => None,
    };
    Ok(SshTarget { user, host, port })
}

fn seconds(value: Option<u64>) -> Option<Duration> {
    value.map(Duration::from_secs)
}

pub fn resolve(flags: FlagLayer, sources: Sources<'_>) -> Result<(Settings, Vec<Warning>)> {
    let env = sources.env;
    let mut warnings = Vec::new();

    let profile_name = flags
        .profile
        .clone()
        .or_else(|| env.var("OWNPG_PROFILE").map(str::to_owned));
    let file = ProfileFile::load(&sources.paths.config_file)?;
    let profile: ProfileEntry = match (&profile_name, &file) {
        (Some(name), Some(file)) => file.profile(name, &sources.paths.config_file)?.clone(),
        (Some(name), None) => {
            return Err(Error::ProfileUnknown {
                name: name.clone(),
                path: sources.paths.config_file.clone(),
                known: Vec::new(),
            });
        }
        (None, _) => ProfileEntry::default(),
    };

    let env_layer = LibpqLayer::from_environment(env);
    let service_name = Pick::new(None, None, profile.service.clone())
        .with_libpq(env_layer.service.clone())
        .resolve();
    let service_layer = match &service_name {
        Some(service) => libpq::service_section(env, &service.value)?.unwrap_or_default(),
        None => LibpqLayer::default(),
    };
    let dsn_text = Pick::new(
        None,
        env.var("OWNPG_DSN").map(str::to_owned),
        profile.dsn.clone(),
    )
    .resolve();
    let dsn_layer = match &dsn_text {
        Some(dsn) => libpq::parse_dsn(&dsn.value)?,
        None => LibpqLayer::default(),
    };
    let libpq_layer = dsn_layer.over(service_layer.over(env_layer));

    let mode = Pick::new(
        flags.mode,
        env_parsed(
            env,
            "OWNPG_MODE",
            Mode::parse,
            "read-only, write-only, or read-write",
        )?,
        profile.mode,
    )
    .or_preset(Mode::ReadOnly);

    let database = Pick::new(
        flags.database.clone(),
        env.var("OWNPG_DATABASE").map(str::to_owned),
        profile.database.clone(),
    )
    .with_libpq(libpq_layer.dbname.clone())
    .resolve()
    .ok_or(Error::DatabaseMissing)?;

    let schema = Pick::new(
        flags.schema.clone(),
        env.var("OWNPG_SCHEMA").map(str::to_owned),
        profile.schema.clone(),
    )
    .or_preset(DEFAULT_SCHEMA.to_owned());

    let host = Pick::new(
        flags.host.clone(),
        env.var("OWNPG_HOST").map(str::to_owned),
        profile.host.clone(),
    )
    .with_libpq(libpq_layer.host.clone())
    .resolve();
    let hostaddr = Pick::new(None, None, profile.hostaddr.clone())
        .with_libpq(libpq_layer.hostaddr.clone())
        .resolve();
    let port = Pick::new(
        flags.port,
        env_u64(env, "OWNPG_PORT")?
            .map(|value| {
                u16::try_from(value)
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| Error::ConfigInvalid {
                        setting: "OWNPG_PORT".to_owned(),
                        value: value.to_string(),
                        detail: "expected a number from 1 to 65535".to_owned(),
                    })
            })
            .transpose()?,
        profile.port,
    )
    .with_libpq(libpq_layer.port)
    .or_preset(DEFAULT_PORT);
    let user = Pick::new(
        flags.user.clone(),
        env.var("OWNPG_USER").map(str::to_owned),
        profile.user.clone(),
    )
    .with_libpq(libpq_layer.user.clone())
    .or_preset(
        env.os_user()
            .map_or_else(|| libpq::DEFAULT_USER.to_owned(), str::to_owned),
    );

    let mut password = Pick::new(
        None,
        env.var("OWNPG_PASSWORD")
            .map(|value| Secret::new(value.to_owned())),
        profile.password.clone().map(Secret::new),
    )
    .resolve();
    if password.is_none()
        && let Some(variable) = &profile.password_env
    {
        password = env
            .var(variable)
            .map(|value| Resolved::new(Secret::new(value.to_owned()), Origin::Profile));
    }
    if password.is_none()
        && profile.password_keychain == Some(true)
        && let (Some(name), Some(lookup)) = (&profile_name, sources.keychain)
    {
        password = lookup(name)?.map(|value| Resolved::new(Secret::new(value), Origin::Profile));
    }
    if password.is_none() {
        password = libpq_layer
            .password
            .clone()
            .map(|value| Resolved::new(value, Origin::Libpq));
    }
    if password.is_none()
        && let Some(path) = libpq::password_file_path(env, libpq_layer.passfile.as_deref())
    {
        match libpq::password_from_file(
            &path,
            host.as_ref().map(|host| host.value.as_str()),
            port.value,
            &database.value,
            &user.value,
        )? {
            PasswordFileOutcome::Found(secret) => {
                password = Some(Resolved::new(secret, Origin::Libpq));
            }
            PasswordFileOutcome::IgnoredPermissions { path, mode } => warnings.push(Warning {
                code: "password_file_ignored",
                message: format!(
                    "password file ignored: permissions ({} has mode {mode:o}; run chmod 600 on it)",
                    path.display()
                ),
            }),
            PasswordFileOutcome::NoMatch | PasswordFileOutcome::Missing => {}
        }
    }

    let sslmode = Pick::new(
        flags.sslmode,
        env_parsed(
            env,
            "OWNPG_SSLMODE",
            SslMode::parse,
            "disable, allow, prefer, require, verify-ca, or verify-full",
        )?,
        profile.sslmode,
    )
    .with_libpq(libpq_layer.sslmode)
    .or_preset(SslMode::Prefer);
    let sslrootcert = Pick::new(
        flags.sslrootcert.clone(),
        env_path(env, "OWNPG_SSLROOTCERT"),
        profile.sslrootcert.clone(),
    )
    .with_libpq(libpq_layer.sslrootcert.clone())
    .resolve();
    let sslcert = Pick::new(
        None,
        env_path(env, "OWNPG_SSLCERT"),
        profile.sslcert.clone(),
    )
    .with_libpq(libpq_layer.sslcert.clone())
    .resolve();
    let sslkey = Pick::new(None, env_path(env, "OWNPG_SSLKEY"), profile.sslkey.clone())
        .with_libpq(libpq_layer.sslkey.clone())
        .resolve();
    let channel_binding = Pick::new(None, None, profile.channel_binding)
        .with_libpq(libpq_layer.channel_binding)
        .or_preset(ChannelBinding::Prefer);
    let connect_timeout = Pick::new(
        None,
        seconds(env_u64(env, "OWNPG_CONNECT_TIMEOUT")?),
        seconds(profile.connect_timeout_seconds),
    )
    .with_libpq(libpq_layer.connect_timeout)
    .or_preset(DEFAULT_CONNECT_TIMEOUT);
    let application_name = Pick::new(None, None, profile.application_name.clone())
        .with_libpq(libpq_layer.application_name.clone())
        .or_preset(format!("ownpg/{}", crate::VERSION));
    let options = Pick::new(None, None, profile.options.clone())
        .with_libpq(libpq_layer.options.clone())
        .resolve();
    let pooled = Pick::new(None, env_bool(env, "OWNPG_POOLED")?, profile.pooled)
        .resolve()
        .map_or(Resolved::preset(None), |value| {
            Resolved::new(Some(value.value), value.origin)
        });

    let limits = LimitSettings {
        statement_timeout: Pick::new(
            None,
            seconds(env_u64(env, "OWNPG_STATEMENT_TIMEOUT")?),
            seconds(profile.statement_timeout_seconds),
        )
        .or_preset(DEFAULT_STATEMENT_TIMEOUT),
        lock_timeout: Pick::new(
            None,
            seconds(env_u64(env, "OWNPG_LOCK_TIMEOUT")?),
            seconds(profile.lock_timeout_seconds),
        )
        .or_preset(DEFAULT_LOCK_TIMEOUT),
        transaction_timeout: Pick::new(
            None,
            seconds(env_u64(env, "OWNPG_TRANSACTION_TIMEOUT")?),
            seconds(profile.transaction_timeout_seconds),
        )
        .or_preset(DEFAULT_TRANSACTION_TIMEOUT),
        handle_expiry: Pick::new(
            None,
            seconds(env_u64(env, "OWNPG_HANDLE_EXPIRY")?),
            seconds(profile.handle_expiry_seconds),
        )
        .or_preset(DEFAULT_HANDLE_EXPIRY),
        row_cap: Pick::new(
            None,
            env_u64(env, "OWNPG_ROW_CAP")?.map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
            profile.row_cap,
        )
        .or_preset(DEFAULT_ROW_CAP),
        byte_cap: Pick::new(
            None,
            env_u64(env, "OWNPG_BYTE_CAP")?.map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
            profile.byte_cap,
        )
        .or_preset(DEFAULT_BYTE_CAP),
    };
    if limits.row_cap.value == 0 || limits.row_cap.value > MAX_ROW_CAP {
        return Err(Error::ConfigInvalid {
            setting: "row_cap".to_owned(),
            value: limits.row_cap.value.to_string(),
            detail: format!("expected 1 to {MAX_ROW_CAP}"),
        });
    }
    if limits.handle_expiry.value.is_zero() {
        return Err(Error::ConfigInvalid {
            setting: "handle_expiry_seconds".to_owned(),
            value: "0".to_owned(),
            detail: "a handle needs an expiry above zero".to_owned(),
        });
    }

    let strict_role = Pick::new(
        flags.strict_role,
        env_bool(env, "OWNPG_STRICT_ROLE")?,
        profile.strict_role,
    )
    .or_preset(false);

    let tools = Pick::new(
        flags.tools.clone(),
        env.var("OWNPG_TOOLS").map(parse_tool_groups).transpose()?,
        profile.tools.clone(),
    )
    .or_preset(Vec::new());
    for group in &tools.value {
        if !group.allowed_in(mode.value) {
            return Err(Error::ConfigInvalid {
                setting: "tools".to_owned(),
                value: group.to_string(),
                detail: format!(
                    "the `{group}` group is not available in {} mode",
                    mode.value
                ),
            });
        }
    }

    let ssh = resolve_ssh(&flags, env, &profile)?;

    let audit = AuditSettings {
        enabled: Pick::new(flags.audit, env_bool(env, "OWNPG_AUDIT")?, profile.audit)
            .or_preset(true),
        path: Pick::new(
            flags.audit_path.clone(),
            env_path(env, "OWNPG_AUDIT_PATH"),
            profile.audit_path.clone(),
        )
        .resolve(),
        max_bytes: Pick::new(
            None,
            env_u64(env, "OWNPG_AUDIT_MAX_BYTES")?,
            profile.audit_max_bytes,
        )
        .or_preset(DEFAULT_AUDIT_MAX_BYTES),
    };

    let no_input = Pick::new(
        flags.no_input,
        env_bool(env, "OWNPG_NO_INPUT")?.or(env_bool(env, "CI")?.filter(|value| *value)),
        profile.no_input,
    )
    .or_preset(false);

    let pg_bindir = Pick::new(
        flags.pg_bindir.clone(),
        env_path(env, "OWNPG_PG_BINDIR"),
        profile.pg_bindir.clone(),
    )
    .resolve();
    let output_dir = Pick::new(
        flags.output_dir.clone(),
        env_path(env, "OWNPG_OUTPUT_DIR"),
        profile.output_dir.clone(),
    )
    .resolve();

    let settings = Settings {
        profile: profile_name,
        mode,
        database,
        schema,
        connection: ConnectionSettings {
            dsn: dsn_text.map(|dsn| Resolved::new(Secret::new(dsn.value), dsn.origin)),
            service: service_name,
            host,
            hostaddr,
            port,
            user,
            password,
            sslmode,
            sslrootcert,
            sslcert,
            sslkey,
            channel_binding,
            connect_timeout,
            application_name,
            options,
            pooled,
        },
        limits,
        strict_role,
        tools,
        ssh,
        audit,
        pg_bindir,
        output_dir,
        no_input,
        paths: sources.paths,
    };
    Ok((settings, warnings))
}

fn resolve_ssh(
    flags: &FlagLayer,
    env: &Environment,
    profile: &ProfileEntry,
) -> Result<Option<SshSettings>> {
    let target = Pick::new(
        flags.ssh.as_deref().map(parse_ssh_target).transpose()?,
        env.var("OWNPG_SSH").map(parse_ssh_target).transpose()?,
        profile.ssh.as_ref().map(|entry| SshTarget {
            user: entry.user.clone(),
            host: entry.host.clone(),
            port: entry.port,
        }),
    )
    .resolve();
    let Some(target) = target else {
        return Ok(None);
    };
    let entry = profile.ssh.clone().unwrap_or_default();
    let origin = target.origin;
    let password = entry
        .password_env
        .as_deref()
        .and_then(|variable| env.var(variable))
        .map(|value| Resolved::new(Secret::new(value.to_owned()), Origin::Profile));
    Ok(Some(SshSettings {
        host: Resolved::new(target.value.host, origin),
        port: target
            .value
            .port
            .map_or_else(|| Resolved::preset(22), |port| Resolved::new(port, origin)),
        user: target
            .value
            .user
            .map(|user| Resolved::new(user, origin))
            .or_else(|| env.os_user().map(|user| Resolved::preset(user.to_owned()))),
        key_file: entry
            .key_file
            .clone()
            .map(|path| Resolved::new(path, Origin::Profile)),
        agent: entry.agent.map_or(Resolved::preset(true), |agent| {
            Resolved::new(agent, Origin::Profile)
        }),
        password,
        trust_new_host: Pick::new(
            flags.ssh_trust_new_host,
            env_bool(env, "OWNPG_SSH_TRUST_NEW_HOST")?,
            entry.trust_new_host,
        )
        .or_preset(false),
        transport: Pick::new(
            flags.ssh_transport,
            env_parsed(
                env,
                "OWNPG_SSH_MODE",
                SshTransport::parse,
                "in-process or system",
            )?,
            entry.transport,
        )
        .or_preset(SshTransport::InProcess),
        jump: entry
            .jump
            .clone()
            .map_or(Resolved::preset(Vec::new()), |jump| {
                Resolved::new(jump, Origin::Profile)
            }),
        known_hosts: entry
            .known_hosts
            .clone()
            .map(|path| Resolved::new(path, Origin::Profile))
            .or_else(|| env.in_home(".ssh/known_hosts").map(Resolved::preset)),
        config_file: entry
            .config_file
            .clone()
            .map(|path| Resolved::new(path, Origin::Profile))
            .or_else(|| env.in_home(".ssh/config").map(Resolved::preset)),
        connect_timeout: entry
            .connect_timeout_seconds
            .map_or(Resolved::preset(Duration::from_secs(10)), |seconds| {
                Resolved::new(Duration::from_secs(seconds), Origin::Profile)
            }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::profile::ProfileFile;
    use crate::error::ErrorId;
    use std::collections::BTreeMap;

    fn sources(dir: &std::path::Path, env: &Environment) -> AppPaths {
        let _ = env;
        AppPaths::from_base(dir.join("config"), dir.join("data"), dir.join("cache"))
    }

    fn resolve_with(
        flags: FlagLayer,
        env: &Environment,
        paths: AppPaths,
    ) -> Result<(Settings, Vec<Warning>)> {
        resolve(
            flags,
            Sources {
                env,
                paths,
                keychain: None,
            },
        )
    }

    fn write_profiles(paths: &AppPaths, file: &ProfileFile) {
        file.save(&paths.config_file).unwrap();
    }

    #[test]
    fn a_bare_run_with_a_database_flag_uses_the_presets() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default().with_os_user("sharkar");
        let paths = sources(dir.path(), &env);
        let flags = FlagLayer {
            database: Some("app".to_owned()),
            ..FlagLayer::default()
        };
        let (settings, warnings) = resolve_with(flags, &env, paths).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(settings.mode.value, Mode::ReadOnly);
        assert_eq!(settings.mode.origin, Origin::Preset);
        assert_eq!(settings.database.value, "app");
        assert_eq!(settings.database.origin, Origin::Flag);
        assert_eq!(settings.schema.value, "public");
        assert_eq!(settings.connection.host, None);
        assert_eq!(settings.connection.port.value, 5432);
        assert_eq!(settings.connection.user.value, "sharkar");
        assert_eq!(settings.connection.user.origin, Origin::Preset);
        assert_eq!(settings.connection.sslmode.value, SslMode::Prefer);
        assert!(settings.connection.password.is_none());
        assert!(settings.audit.enabled.value);
        assert_eq!(settings.loaded_groups(), Vec::new());
    }

    #[test]
    fn no_database_anywhere_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default();
        let paths = sources(dir.path(), &env);
        let error = resolve_with(FlagLayer::default(), &env, paths).unwrap_err();
        assert_eq!(error.id(), ErrorId::DatabaseMissing);
        assert!(error.remedy().contains("--database"));
    }

    #[test]
    fn a_flag_beats_the_environment_which_beats_the_profile_which_beats_libpq() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_MODE", "read-write")
            .with_var("OWNPG_PROFILE", "local")
            .with_var("PGHOST", "env-host")
            .with_var("PGDATABASE", "env-db");
        let paths = sources(dir.path(), &env);
        let mut file = ProfileFile::default();
        file.profiles.insert(
            "local".to_owned(),
            ProfileEntry {
                mode: Some(Mode::WriteOnly),
                host: Some("profile-host".to_owned()),
                schema: Some("app".to_owned()),
                ..ProfileEntry::default()
            },
        );
        write_profiles(&paths, &file);
        let flags = FlagLayer {
            mode: Some(Mode::ReadOnly),
            ..FlagLayer::default()
        };
        let (settings, _) = resolve_with(flags, &env, paths).unwrap();
        assert_eq!(settings.mode.value, Mode::ReadOnly);
        assert_eq!(settings.mode.origin, Origin::Flag);
        assert_eq!(settings.schema.value, "app");
        assert_eq!(settings.schema.origin, Origin::Profile);
        assert_eq!(
            settings.connection.host.as_ref().unwrap().value,
            "profile-host"
        );
        assert_eq!(settings.database.value, "env-db");
        assert_eq!(settings.database.origin, Origin::Libpq);
    }

    #[test]
    fn an_exported_preference_never_stands_in_for_a_flag_in_the_origin() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_MODE", "read-write")
            .with_var("OWNPG_DATABASE", "app");
        let paths = sources(dir.path(), &env);
        let (settings, _) = resolve_with(FlagLayer::default(), &env, paths).unwrap();
        assert_eq!(settings.mode.value, Mode::ReadWrite);
        assert_eq!(settings.mode.origin, Origin::Environment);
    }

    #[test]
    fn a_named_profile_that_does_not_exist_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default();
        let paths = sources(dir.path(), &env);
        let flags = FlagLayer {
            profile: Some("staging".to_owned()),
            database: Some("x".to_owned()),
            ..FlagLayer::default()
        };
        let error = resolve_with(flags, &env, paths).unwrap_err();
        assert_eq!(error.id(), ErrorId::ProfileUnknown);
    }

    #[test]
    fn the_service_file_wins_over_the_environment_and_the_dsn_wins_over_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let service_file = dir.path().join("service.conf");
        std::fs::write(
            &service_file,
            "[staging]\nhost=service-host\nport=5433\nuser=svc\n",
        )
        .unwrap();
        let env = Environment::default()
            .with_var("PGSERVICE", "staging")
            .with_var("PGSERVICEFILE", service_file.to_str().unwrap())
            .with_var("PGHOST", "env-host")
            .with_var("PGUSER", "env-user")
            .with_var("OWNPG_DSN", "postgresql://dsn-user@dsn-host/dsn-db");
        let paths = sources(dir.path(), &env);
        let (settings, _) = resolve_with(FlagLayer::default(), &env, paths).unwrap();
        assert_eq!(settings.connection.host.as_ref().unwrap().value, "dsn-host");
        assert_eq!(settings.connection.user.value, "dsn-user");
        assert_eq!(settings.connection.port.value, 5433);
        assert_eq!(settings.database.value, "dsn-db");
        assert_eq!(
            settings.connection.service.as_ref().unwrap().value,
            "staging"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_password_file_supplies_a_password_and_a_loose_one_only_warns() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let pgpass = dir.path().join("pgpass");
        std::fs::write(&pgpass, "db.internal:5432:app:reader:from-file\n").unwrap();
        std::fs::set_permissions(&pgpass, std::fs::Permissions::from_mode(0o600)).unwrap();
        let env = Environment::default()
            .with_var("PGPASSFILE", pgpass.to_str().unwrap())
            .with_var("PGHOST", "db.internal")
            .with_var("PGUSER", "reader")
            .with_var("PGDATABASE", "app");
        let paths = sources(dir.path(), &env);
        let (settings, warnings) = resolve_with(FlagLayer::default(), &env, paths.clone()).unwrap();
        assert_eq!(
            settings
                .connection
                .password
                .as_ref()
                .map(|value| value.value.expose()),
            Some("from-file")
        );
        assert!(warnings.is_empty());
        std::fs::set_permissions(&pgpass, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (settings, warnings) = resolve_with(FlagLayer::default(), &env, paths).unwrap();
        assert!(settings.connection.password.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0]
                .message
                .starts_with("password file ignored: permissions")
        );
    }

    #[test]
    fn a_profile_password_and_a_password_env_and_the_keychain_are_all_honored() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_PROFILE", "plain")
            .with_var("APP_DB_PASSWORD", "from-env");
        let paths = sources(dir.path(), &env);
        let mut file = ProfileFile::default();
        file.profiles.insert(
            "plain".to_owned(),
            ProfileEntry {
                database: Some("app".to_owned()),
                password: Some("plain-text".to_owned()),
                ..ProfileEntry::default()
            },
        );
        file.profiles.insert(
            "env".to_owned(),
            ProfileEntry {
                database: Some("app".to_owned()),
                password_env: Some("APP_DB_PASSWORD".to_owned()),
                ..ProfileEntry::default()
            },
        );
        file.profiles.insert(
            "chain".to_owned(),
            ProfileEntry {
                database: Some("app".to_owned()),
                password_keychain: Some(true),
                ..ProfileEntry::default()
            },
        );
        write_profiles(&paths, &file);
        let (settings, _) = resolve_with(FlagLayer::default(), &env, paths.clone()).unwrap();
        assert_eq!(
            settings
                .connection
                .password
                .as_ref()
                .unwrap()
                .value
                .expose(),
            "plain-text"
        );
        let flags = FlagLayer {
            profile: Some("env".to_owned()),
            ..FlagLayer::default()
        };
        let (settings, _) = resolve_with(flags, &env, paths.clone()).unwrap();
        assert_eq!(
            settings
                .connection
                .password
                .as_ref()
                .unwrap()
                .value
                .expose(),
            "from-env"
        );
        let lookup = |name: &str| -> Result<Option<String>> {
            Ok((name == "chain").then(|| "from-keychain".to_owned()))
        };
        let flags = FlagLayer {
            profile: Some("chain".to_owned()),
            ..FlagLayer::default()
        };
        let (settings, _) = resolve(
            flags,
            Sources {
                env: &env,
                paths,
                keychain: Some(&lookup),
            },
        )
        .unwrap();
        assert_eq!(
            settings
                .connection
                .password
                .as_ref()
                .unwrap()
                .value
                .expose(),
            "from-keychain"
        );
    }

    #[test]
    fn a_bad_environment_value_names_the_variable() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_DATABASE", "app")
            .with_var("OWNPG_STRICT_ROLE", "maybe");
        let paths = sources(dir.path(), &env);
        let error = resolve_with(FlagLayer::default(), &env, paths).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigInvalid);
        assert!(error.to_string().contains("OWNPG_STRICT_ROLE"));
    }

    #[test]
    fn a_tool_group_the_mode_forbids_is_refused_at_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default().with_var("OWNPG_DATABASE", "app");
        let paths = sources(dir.path(), &env);
        let flags = FlagLayer {
            tools: Some(vec![ToolGroup::Ddl]),
            ..FlagLayer::default()
        };
        let error = resolve_with(flags, &env, paths).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigInvalid);
        assert!(error.to_string().contains("read-only"));
    }

    #[test]
    fn write_groups_load_in_read_write_mode_and_named_groups_join_them() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_DATABASE", "app")
            .with_var("OWNPG_MODE", "read-write")
            .with_var("OWNPG_TOOLS", "monitoring, ddl");
        let paths = sources(dir.path(), &env);
        let (settings, _) = resolve_with(FlagLayer::default(), &env, paths).unwrap();
        assert_eq!(
            settings.loaded_groups(),
            vec![
                ToolGroup::Write,
                ToolGroup::Transactions,
                ToolGroup::Ddl,
                ToolGroup::Monitoring
            ]
        );
    }

    #[test]
    fn a_row_cap_past_the_ceiling_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_DATABASE", "app")
            .with_var("OWNPG_ROW_CAP", "5000");
        let paths = sources(dir.path(), &env);
        let error = resolve_with(FlagLayer::default(), &env, paths).unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigInvalid);
        assert!(error.to_string().contains("row_cap"));
    }

    #[test]
    fn the_ci_variable_implies_no_input_unless_a_flag_says_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_DATABASE", "app")
            .with_var("CI", "true");
        let paths = sources(dir.path(), &env);
        let (settings, _) = resolve_with(FlagLayer::default(), &env, paths.clone()).unwrap();
        assert!(settings.no_input.value);
        assert_eq!(settings.no_input.origin, Origin::Environment);
        let flags = FlagLayer {
            no_input: Some(false),
            ..FlagLayer::default()
        };
        let (settings, _) = resolve_with(flags, &env, paths).unwrap();
        assert!(!settings.no_input.value);
    }

    #[test]
    fn an_ssh_flag_parses_user_host_and_port() {
        let target = parse_ssh_target("deploy@bastion.example.com:2222").unwrap();
        assert_eq!(target.user.as_deref(), Some("deploy"));
        assert_eq!(target.host, "bastion.example.com");
        assert_eq!(target.port, Some(2222));
        let bare = parse_ssh_target("bastion").unwrap();
        assert_eq!(bare.user, None);
        assert_eq!(bare.port, None);
        let ipv6 = parse_ssh_target("root@[fe80::1]:22").unwrap();
        assert_eq!(ipv6.host, "fe80::1");
        assert!(parse_ssh_target("deploy@").is_err());
        assert!(parse_ssh_target("host:0").is_err());
    }

    #[test]
    fn ssh_settings_come_from_the_flag_with_profile_details_underneath() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default()
            .with_var("OWNPG_DATABASE", "app")
            .with_var("OWNPG_PROFILE", "staging")
            .with_home("/home/u".into())
            .with_os_user("u");
        let paths = sources(dir.path(), &env);
        let mut file = ProfileFile::default();
        file.profiles.insert(
            "staging".to_owned(),
            ProfileEntry {
                ssh: Some(super::super::profile::SshEntry {
                    host: "profile-bastion".to_owned(),
                    key_file: Some("/keys/id".into()),
                    trust_new_host: Some(true),
                    ..Default::default()
                }),
                ..ProfileEntry::default()
            },
        );
        write_profiles(&paths, &file);
        let flags = FlagLayer {
            ssh: Some("deploy@flag-bastion".to_owned()),
            ssh_trust_new_host: Some(false),
            ..FlagLayer::default()
        };
        let (settings, _) = resolve_with(flags, &env, paths).unwrap();
        let ssh = settings.ssh.unwrap();
        assert_eq!(ssh.host.value, "flag-bastion");
        assert_eq!(ssh.host.origin, Origin::Flag);
        assert_eq!(ssh.user.unwrap().value, "deploy");
        assert_eq!(ssh.port.value, 22);
        assert_eq!(ssh.key_file.unwrap().value, PathBuf::from("/keys/id"));
        assert!(!ssh.trust_new_host.value);
        assert_eq!(ssh.transport.value, SshTransport::InProcess);
        assert_eq!(
            ssh.known_hosts.unwrap().value,
            PathBuf::from("/home/u/.ssh/known_hosts")
        );
    }

    #[test]
    fn tool_group_lists_parse_and_dedupe() {
        assert_eq!(
            parse_tool_groups("ddl, roles,ddl").unwrap(),
            vec![ToolGroup::Ddl, ToolGroup::Roles]
        );
        assert_eq!(
            parse_tool_groups("nope").unwrap_err().id(),
            ErrorId::ConfigInvalid
        );
        let empty: BTreeMap<String, String> = BTreeMap::new();
        assert!(empty.is_empty());
    }
}
