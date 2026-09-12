use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use ownpg_core::config::{
    AppPaths, Environment, FlagLayer, Mode, SshTransport, SslMode, parse_tool_groups,
};
use ownpg_core::connect::ssh::Hints;
use ownpg_core::{Error, Result};

use crate::cli::{ConnectionArgs, GlobalArgs, ModeArg, ServeArgs, SshTransportArg, SslModeArg};

pub(crate) const KEYCHAIN_SERVICE: &str = "ownpg";

#[derive(Debug, Clone)]
pub(crate) struct Process {
    pub env: Environment,
    pub paths: AppPaths,
}

pub(crate) fn detect(global: &GlobalArgs) -> Result<Process> {
    let vars: BTreeMap<String, String> = std::env::vars().collect();
    let home = etcetera::home_dir().ok();
    let os_user = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|name| vars.get(*name).cloned())
        .filter(|user| !user.trim().is_empty());
    let env = Environment::new(vars, home, os_user);
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "bd".to_owned(),
        author: "devops".to_owned(),
        app_name: "ownpg".to_owned(),
    })
    .map_err(|error| Error::ConfigInvalid {
        setting: "home directory".to_owned(),
        value: String::new(),
        detail: error.to_string(),
    })?;
    let mut paths = AppPaths::from_base(
        strategy.config_dir(),
        strategy.data_dir(),
        strategy.cache_dir(),
    );
    if let Some(config) = &global.config {
        paths = paths.with_config_file(config.clone());
    }
    Ok(Process { env, paths })
}

pub(crate) fn keychain_entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(|error| Error::ConfigInvalid {
        setting: "keychain".to_owned(),
        value: account.to_owned(),
        detail: error.to_string(),
    })
}

pub(crate) fn keychain_lookup(account: &str) -> Result<Option<String>> {
    match keychain_entry(account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(Error::ConfigInvalid {
            setting: "keychain".to_owned(),
            value: account.to_owned(),
            detail: error.to_string(),
        }),
    }
}

pub(crate) fn ssh_hints(env: &Environment) -> Hints {
    Hints {
        home: env.home().map(Path::to_path_buf),
        agent_socket: env.var("SSH_AUTH_SOCK").map(PathBuf::from),
        os_user: env.os_user().map(str::to_owned),
    }
}

pub(crate) fn flag_layer(
    connection: &ConnectionArgs,
    global: &GlobalArgs,
    serve: Option<&ServeArgs>,
) -> Result<FlagLayer> {
    if connection.ssh_transport == Some(SshTransportArg::System) && connection.ssh_trust_new_host {
        return Err(Error::ArgumentInvalid {
            argument: "--ssh-trust-new-host".to_owned(),
            detail: "the system ssh command manages its own known hosts; drop the flag or use --ssh-transport in-process".to_owned(),
        });
    }
    let tools = if connection.tools.is_empty() {
        None
    } else {
        Some(parse_tool_groups(&connection.tools.join(","))?)
    };
    Ok(FlagLayer {
        profile: connection.profile.clone(),
        mode: connection.mode.map(mode_of),
        database: connection.database.clone(),
        schema: connection.schema.clone(),
        host: connection.host.clone(),
        port: connection.port,
        user: connection.user.clone(),
        sslmode: connection.sslmode.map(sslmode_of),
        sslrootcert: connection.sslrootcert.clone(),
        tools,
        strict_role: connection.strict_role.then_some(true),
        ssh: connection.ssh.clone(),
        ssh_transport: connection.ssh_transport.map(|mode| match mode {
            SshTransportArg::InProcess => SshTransport::InProcess,
            SshTransportArg::System => SshTransport::System,
        }),
        ssh_trust_new_host: connection.ssh_trust_new_host.then_some(true),
        no_input: global.no_input.then_some(true),
        audit: serve.and_then(|serve| serve.no_audit.then_some(false)),
        audit_path: serve.and_then(|serve| serve.audit_path.clone()),
        pg_bindir: serve.and_then(|serve| serve.pg_bindir.clone()),
        output_dir: serve.and_then(|serve| serve.output_dir.clone()),
        http: ownpg_core::config::HttpFlags {
            enabled: serve.is_some_and(|serve| serve.http),
            bind: serve.and_then(|serve| serve.bind.clone()),
            auth: serve.and_then(|serve| serve.auth.map(auth_of)),
        },
    })
}

pub(crate) const fn auth_of(auth: crate::cli::AuthArg) -> ownpg_core::config::AuthMode {
    match auth {
        crate::cli::AuthArg::None => ownpg_core::config::AuthMode::None,
        crate::cli::AuthArg::Bearer => ownpg_core::config::AuthMode::Bearer,
        crate::cli::AuthArg::Oauth => ownpg_core::config::AuthMode::Oauth,
    }
}

pub(crate) const fn mode_of(mode: ModeArg) -> Mode {
    match mode {
        ModeArg::ReadOnly => Mode::ReadOnly,
        ModeArg::WriteOnly => Mode::WriteOnly,
        ModeArg::ReadWrite => Mode::ReadWrite,
    }
}

pub(crate) const fn sslmode_of(mode: SslModeArg) -> SslMode {
    match mode {
        SslModeArg::Disable => SslMode::Disable,
        SslModeArg::Allow => SslMode::Allow,
        SslModeArg::Prefer => SslMode::Prefer,
        SslModeArg::Require => SslMode::Require,
        SslModeArg::VerifyCa => SslMode::VerifyCa,
        SslModeArg::VerifyFull => SslMode::VerifyFull,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global() -> GlobalArgs {
        GlobalArgs {
            verbose: 0,
            quiet: false,
            no_input: false,
            log_format: crate::cli::LogFormatArg::Text,
            log_file: None,
            config: None,
        }
    }

    #[test]
    fn the_no_input_flag_is_passed_through_and_system_ssh_refuses_trust_on_first_use() {
        let mut flagged = global();
        flagged.no_input = true;
        let layer = flag_layer(&ConnectionArgs::default(), &flagged, None).unwrap();
        assert_eq!(layer.no_input, Some(true));
        let plain = flag_layer(&ConnectionArgs::default(), &global(), None).unwrap();
        assert_eq!(plain.no_input, None);
        let refused = flag_layer(
            &ConnectionArgs {
                ssh_transport: Some(SshTransportArg::System),
                ssh_trust_new_host: true,
                ..ConnectionArgs::default()
            },
            &global(),
            None,
        )
        .unwrap_err();
        assert_eq!(refused.id().as_str(), "argument.invalid");
    }

    #[test]
    fn tool_groups_and_modes_map_onto_the_core_types() {
        let layer = flag_layer(
            &ConnectionArgs {
                tools: vec!["ddl".to_owned(), "roles".to_owned()],
                mode: Some(ModeArg::WriteOnly),
                ..ConnectionArgs::default()
            },
            &global(),
            None,
        )
        .unwrap();
        assert_eq!(layer.mode, Some(Mode::WriteOnly));
        assert_eq!(layer.tools.map(|groups| groups.len()), Some(2));
        assert_eq!(sslmode_of(SslModeArg::VerifyFull), SslMode::VerifyFull);
    }
}
