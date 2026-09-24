use std::io::{IsTerminal, Read};

use ownpg_core::config::describe::describe;
use ownpg_core::config::keychain::{password_account_for, ssh_secret_account_for};
use ownpg_core::config::profile::{ProfileFile, create_private, lock_profiles, write_private};
use ownpg_core::config::{FlagLayer, Secret, Settings, Sources, resolve};
use ownpg_core::{Error, ExitClass, Result};

use crate::cli::{ConfigCommand, GlobalArgs, OutputFormatArg};
use crate::context::{self, Process, keychain_entry};
use crate::output::{emit, stdout_error};

pub(crate) fn run(
    global: &GlobalArgs,
    command: &ConfigCommand,
    process: &Process,
) -> Result<ExitClass> {
    match command {
        ConfigCommand::Show { connection, format } => {
            let flags = context::flag_layer(connection, global, None)?;
            let lookup = context::keychain_lookup;
            let (settings, warnings) = resolve(
                flags,
                Sources {
                    env: &process.env,
                    paths: process.paths.clone(),
                    keychain: Some(&lookup),
                },
            )?;
            for warning in &warnings {
                tracing::warn!(code = warning.code, "{}", warning.message);
            }
            let lines = describe(&settings);
            match format {
                OutputFormatArg::Json => {
                    let document = serde_json::json!({
                        "format_version": crate::doctor::FORMAT_VERSION,
                        "settings": lines,
                    });
                    let rendered = serde_json::to_string_pretty(&document).map_err(|error| {
                        Error::ProtocolFailed {
                            detail: format!("the settings could not be serialized: {error}"),
                        }
                    })?;
                    emit(|out| writeln!(out, "{rendered}")).map_err(stdout_error)?;
                }
                OutputFormatArg::Text => {
                    emit(|out| {
                        for line in &lines {
                            writeln!(out, "{} = {} ({})", line.name, line.value, line.origin)?;
                        }
                        Ok(())
                    })
                    .map_err(stdout_error)?;
                }
            }
            Ok(ExitClass::Success)
        }
        ConfigCommand::Path { format } => {
            let paths = &process.paths;
            match format {
                OutputFormatArg::Json => {
                    let document = serde_json::json!({
                        "format_version": crate::doctor::FORMAT_VERSION,
                        "profiles": paths.config_file,
                        "data": paths.data_dir,
                        "logs": paths.log_dir,
                    });
                    let rendered = serde_json::to_string_pretty(&document).map_err(|error| {
                        Error::ProtocolFailed {
                            detail: format!("the paths could not be serialized: {error}"),
                        }
                    })?;
                    emit(|out| writeln!(out, "{rendered}")).map_err(stdout_error)?;
                }
                OutputFormatArg::Text => {
                    emit(|out| {
                        writeln!(out, "profiles: {}", paths.config_file.display())?;
                        writeln!(out, "data: {}", paths.data_dir.display())?;
                        writeln!(out, "logs: {}", paths.log_dir.display())
                    })
                    .map_err(stdout_error)?;
                }
            }
            Ok(ExitClass::Success)
        }
        ConfigCommand::Init { force, dry_run } => init(process, *force, *dry_run),
        ConfigCommand::SetPassword { profile, dry_run } => {
            if *dry_run {
                return preview(
                    process,
                    profile,
                    "store the password in the keychain and set password_keychain = true",
                );
            }
            set_password(global, process, profile)
        }
        ConfigCommand::UnsetPassword { profile, dry_run } => {
            if *dry_run {
                return preview(
                    process,
                    profile,
                    "remove the password from the keychain and clear password_keychain",
                );
            }
            unset_password(process, profile)
        }
        ConfigCommand::SetSshPassphrase { profile, dry_run } => {
            if *dry_run {
                return preview(
                    process,
                    profile,
                    "store the SSH passphrase in the keychain and set ssh.passphrase_keychain = true",
                );
            }
            set_ssh_passphrase(global, process, profile)
        }
        ConfigCommand::UnsetSshPassphrase { profile, dry_run } => {
            if *dry_run {
                return preview(
                    process,
                    profile,
                    "remove the SSH passphrase from the keychain and clear ssh.passphrase_keychain",
                );
            }
            unset_ssh_passphrase(process, profile)
        }
    }
}

fn preview(process: &Process, profile: &str, action: &str) -> Result<ExitClass> {
    load_profiles(process, profile)?;
    emit(|out| {
        writeln!(
            out,
            "would {action} for profile `{profile}` in {}",
            process.paths.config_file.display()
        )
    })
    .map_err(stdout_error)?;
    Ok(ExitClass::Success)
}

fn init(process: &Process, force: bool, dry_run: bool) -> Result<ExitClass> {
    let path = &process.paths.config_file;
    let text = ownpg_core::config::profile::PROFILE_TEMPLATE;
    if dry_run {
        emit(|out| out.write_all(text.as_bytes())).map_err(stdout_error)?;
        return Ok(ExitClass::Success);
    }
    let _lock = lock_profiles(path)?;
    if force {
        if let Ok(previous) = std::fs::read(path) {
            let mut backup = path.as_os_str().to_owned();
            backup.push(".bak");
            let backup = std::path::PathBuf::from(backup);
            write_private(&backup, &previous)?;
            tracing::info!(backup = %backup.display(), "the previous profile file was saved; keychain entries it used are kept");
        }
        write_private(path, text.as_bytes())?;
    } else {
        create_private(path, text.as_bytes())?;
    }
    tracing::info!(path = %path.display(), "profile file written");
    emit(|out| writeln!(out, "{}", path.display())).map_err(stdout_error)?;
    Ok(ExitClass::Success)
}

fn load_profiles(process: &Process, profile: &str) -> Result<ProfileFile> {
    let path = &process.paths.config_file;
    let file = ProfileFile::load(path)?.ok_or_else(|| Error::ProfileUnknown {
        name: profile.to_owned(),
        path: path.clone(),
        known: Vec::new(),
    })?;
    file.profile(profile, path)?;
    Ok(file)
}

fn read_secret(global: &GlobalArgs, process: &Process) -> Result<Secret> {
    let no_input =
        global.no_input || ownpg_core::config::no_input_from_env(&process.env)?.unwrap_or(false);
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        if no_input {
            return Err(Error::ArgumentInvalid {
                argument: "--no-input".to_owned(),
                detail: "a password prompt is needed; pipe the password on stdin instead"
                    .to_owned(),
            });
        }
        return rpassword::prompt_password("Password (not echoed): ")
            .map(Secret::new)
            .map_err(|source| Error::InputUnreadable {
                stream: "the terminal".to_owned(),
                source,
            });
    }
    let mut text = String::new();
    stdin
        .lock()
        .read_to_string(&mut text)
        .map_err(|source| Error::InputUnreadable {
            stream: "stdin".to_owned(),
            source,
        })?;
    let trimmed_len = text.trim_end_matches(['\r', '\n']).len();
    text.truncate(trimmed_len);
    Ok(Secret::new(text))
}

fn resolved_profile(process: &Process, profile: &str) -> Result<Settings> {
    let (settings, warnings) = resolve(
        FlagLayer {
            profile: Some(profile.to_owned()),
            database: Some(ACCOUNT_PROBE_DATABASE.to_owned()),
            ..FlagLayer::default()
        },
        Sources {
            env: &process.env,
            paths: process.paths.clone(),
            keychain: None,
        },
    )?;
    for warning in warnings
        .iter()
        .filter(|warning| warning.code != "keychain_entry_missing")
    {
        tracing::warn!(code = warning.code, "{}", warning.message);
    }
    Ok(settings)
}

const ACCOUNT_PROBE_DATABASE: &str = "postgres";

fn set_password(global: &GlobalArgs, process: &Process, profile: &str) -> Result<ExitClass> {
    let _lock = lock_profiles(&process.paths.config_file)?;
    let mut file = load_profiles(process, profile)?;
    let password = read_secret(global, process)?;
    if password.expose().is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "password".to_owned(),
            detail: "an empty password was given".to_owned(),
        });
    }
    let account = password_account_for(&resolved_profile(process, profile)?);
    if let Some(entry) = file.profiles.get_mut(profile) {
        entry.password = None;
        entry.password_keychain = Some(true);
    }
    store_then_save(
        process,
        &file,
        &account,
        "password_keychain",
        password.expose(),
    )?;
    tracing::info!(profile, %account, "password stored in the platform keychain");
    Ok(ExitClass::Success)
}

fn store_then_save(
    process: &Process,
    file: &ProfileFile,
    account: &str,
    setting: &str,
    secret: &str,
) -> Result<()> {
    let previous = context::keychain_lookup(account)?;
    store_secret(account, setting, secret)?;
    if let Err(error) = file.save(&process.paths.config_file) {
        let restored = match &previous {
            Some(old) => store_secret(account, setting, old),
            None => forget_secret(account, setting),
        };
        if let Err(restore_error) = restored {
            return Err(Error::ConfigInvalid {
                setting: setting.to_owned(),
                value: account.to_owned(),
                detail: format!(
                    "the profile file could not be saved ({error}), and the keychain entry could not be put back ({restore_error}); run the command again"
                ),
            });
        }
        return Err(error);
    }
    Ok(())
}

fn store_secret(account: &str, setting: &str, secret: &str) -> Result<()> {
    keychain_entry(account)?
        .set_password(secret)
        .map_err(|error| Error::ConfigInvalid {
            setting: setting.to_owned(),
            value: account.to_owned(),
            detail: error.to_string(),
        })
}

fn forget_secret(account: &str, setting: &str) -> Result<()> {
    match keychain_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(Error::ConfigInvalid {
            setting: setting.to_owned(),
            value: account.to_owned(),
            detail: error.to_string(),
        }),
    }
}

fn ssh_account(process: &Process, profile: &str) -> Result<String> {
    ssh_secret_account_for(&resolved_profile(process, profile)?).ok_or_else(|| {
        Error::ArgumentInvalid {
            argument: "profile".to_owned(),
            detail: format!("profile `{profile}` has no SSH route"),
        }
    })
}

fn set_ssh_passphrase(global: &GlobalArgs, process: &Process, profile: &str) -> Result<ExitClass> {
    let _lock = lock_profiles(&process.paths.config_file)?;
    let mut file = load_profiles(process, profile)?;
    if file
        .profiles
        .get(profile)
        .is_none_or(|entry| entry.ssh.is_none())
    {
        return Err(Error::ArgumentInvalid {
            argument: "profile".to_owned(),
            detail: format!("profile `{profile}` has no [profiles.{profile}.ssh] section"),
        });
    }
    let passphrase = read_secret(global, process)?;
    if passphrase.expose().is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "passphrase".to_owned(),
            detail: "an empty passphrase was given".to_owned(),
        });
    }
    let account = ssh_account(process, profile)?;
    if let Some(ssh) = file
        .profiles
        .get_mut(profile)
        .and_then(|entry| entry.ssh.as_mut())
    {
        ssh.passphrase_keychain = Some(true);
    }
    store_then_save(
        process,
        &file,
        &account,
        "passphrase_keychain",
        passphrase.expose(),
    )?;
    tracing::info!(profile, %account, "ssh passphrase stored in the platform keychain");
    Ok(ExitClass::Success)
}

fn unset_ssh_passphrase(process: &Process, profile: &str) -> Result<ExitClass> {
    let _lock = lock_profiles(&process.paths.config_file)?;
    let mut file = load_profiles(process, profile)?;
    let account = ssh_account(process, profile)?;
    forget_secret(&account, "passphrase_keychain")?;
    if let Some(ssh) = file
        .profiles
        .get_mut(profile)
        .and_then(|entry| entry.ssh.as_mut())
    {
        ssh.passphrase_keychain = None;
    }
    file.save(&process.paths.config_file)?;
    tracing::info!(profile, %account, "ssh passphrase removed from the platform keychain");
    Ok(ExitClass::Success)
}

fn unset_password(process: &Process, profile: &str) -> Result<ExitClass> {
    let _lock = lock_profiles(&process.paths.config_file)?;
    let mut file = load_profiles(process, profile)?;
    let account = password_account_for(&resolved_profile(process, profile)?);
    forget_secret(&account, "password_keychain")?;
    if let Some(entry) = file.profiles.get_mut(profile) {
        entry.password_keychain = None;
    }
    file.save(&process.paths.config_file)?;
    tracing::info!(profile, %account, "password removed from the platform keychain");
    Ok(ExitClass::Success)
}
