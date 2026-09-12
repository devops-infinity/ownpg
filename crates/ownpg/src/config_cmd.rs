use std::io::{IsTerminal, Read};

use ownpg_core::config::describe::describe;
use ownpg_core::config::profile::{ProfileFile, write_private};
use ownpg_core::config::{Secret, Sources, keychain_account, resolve, ssh_keychain_account};
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
                        "cache": paths.cache_dir,
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
                        writeln!(out, "cache: {}", paths.cache_dir.display())?;
                        writeln!(out, "logs: {}", paths.log_dir.display())
                    })
                    .map_err(stdout_error)?;
                }
            }
            Ok(ExitClass::Success)
        }
        ConfigCommand::Init { force, dry_run } => init(process, *force, *dry_run),
        ConfigCommand::SetPassword { profile } => set_password(global, process, profile),
        ConfigCommand::UnsetPassword { profile } => unset_password(process, profile),
        ConfigCommand::SetSshPassphrase { profile } => set_ssh_passphrase(global, process, profile),
        ConfigCommand::UnsetSshPassphrase { profile } => unset_ssh_passphrase(process, profile),
        ConfigCommand::CacheClear => cache_clear(process),
    }
}

fn init(process: &Process, force: bool, dry_run: bool) -> Result<ExitClass> {
    let path = &process.paths.config_file;
    let text = ownpg_core::config::profile::TEMPLATE;
    if dry_run {
        emit(|out| out.write_all(text.as_bytes())).map_err(stdout_error)?;
        return Ok(ExitClass::Success);
    }
    if path.exists() && !force {
        return Err(Error::ConfigUnwritable {
            path: path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "the file exists; pass --force to replace it",
            ),
        });
    }
    write_private(path, text.as_bytes())?;
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

fn read_password(global: &GlobalArgs, process: &Process) -> Result<Secret> {
    let no_input = global.no_input
        || process.env.var("OWNPG_NO_INPUT").is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        || ownpg_core::config::ci_says_no_input(&process.env).unwrap_or(false);
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
                source_name: "the terminal".to_owned(),
                source,
            });
    }
    let mut text = String::new();
    stdin
        .lock()
        .read_to_string(&mut text)
        .map_err(|source| Error::InputUnreadable {
            source_name: "stdin".to_owned(),
            source,
        })?;
    let trimmed = text.trim_end_matches(['\r', '\n']).len();
    text.truncate(trimmed);
    Ok(Secret::new(text))
}

fn set_password(global: &GlobalArgs, process: &Process, profile: &str) -> Result<ExitClass> {
    let mut file = load_profiles(process, profile)?;
    let password = read_password(global, process)?;
    if password.expose().is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "password".to_owned(),
            detail: "an empty password was given".to_owned(),
        });
    }
    let previous = file.profiles.get(profile).cloned();
    if let Some(entry) = file.profiles.get_mut(profile) {
        entry.password = None;
        entry.password_keychain = Some(true);
    }
    file.save(&process.paths.config_file)?;
    if let Err(error) = store_secret(
        &keychain_account(profile),
        "password_keychain",
        password.expose(),
    ) {
        if let Some(previous) = previous {
            file.profiles.insert(profile.to_owned(), previous);
            let _ = file.save(&process.paths.config_file);
        }
        return Err(error);
    }
    tracing::info!(profile, "password stored in the platform keychain");
    Ok(ExitClass::Success)
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

fn set_ssh_passphrase(global: &GlobalArgs, process: &Process, profile: &str) -> Result<ExitClass> {
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
    let passphrase = read_password(global, process)?;
    if passphrase.expose().is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "passphrase".to_owned(),
            detail: "an empty passphrase was given".to_owned(),
        });
    }
    let previous = file.profiles.get(profile).cloned();
    if let Some(ssh) = file
        .profiles
        .get_mut(profile)
        .and_then(|entry| entry.ssh.as_mut())
    {
        ssh.passphrase_keychain = Some(true);
    }
    file.save(&process.paths.config_file)?;
    if let Err(error) = store_secret(
        &ssh_keychain_account(profile),
        "passphrase_keychain",
        passphrase.expose(),
    ) {
        if let Some(previous) = previous {
            file.profiles.insert(profile.to_owned(), previous);
            let _ = file.save(&process.paths.config_file);
        }
        return Err(error);
    }
    tracing::info!(profile, "ssh passphrase stored in the platform keychain");
    Ok(ExitClass::Success)
}

fn unset_ssh_passphrase(process: &Process, profile: &str) -> Result<ExitClass> {
    let mut file = load_profiles(process, profile)?;
    if let Some(ssh) = file
        .profiles
        .get_mut(profile)
        .and_then(|entry| entry.ssh.as_mut())
    {
        ssh.passphrase_keychain = None;
    }
    file.save(&process.paths.config_file)?;
    forget_secret(&ssh_keychain_account(profile), "passphrase_keychain")?;
    tracing::info!(profile, "ssh passphrase removed from the platform keychain");
    Ok(ExitClass::Success)
}

fn unset_password(process: &Process, profile: &str) -> Result<ExitClass> {
    let mut file = load_profiles(process, profile)?;
    if let Some(entry) = file.profiles.get_mut(profile) {
        entry.password_keychain = None;
    }
    file.save(&process.paths.config_file)?;
    forget_secret(&keychain_account(profile), "password_keychain")?;
    tracing::info!(profile, "password removed from the platform keychain");
    Ok(ExitClass::Success)
}

fn cache_clear(process: &Process) -> Result<ExitClass> {
    let cache = &process.paths.cache_dir;
    let mut removed = 0usize;
    if cache.is_dir() {
        let entries = std::fs::read_dir(cache).map_err(|source| Error::OutputUnwritable {
            target: cache.display().to_string(),
            source,
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let outcome = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            outcome.map_err(|source| Error::OutputUnwritable {
                target: path.display().to_string(),
                source,
            })?;
            removed += 1;
        }
    }
    emit(|out| writeln!(out, "removed {removed} entries from {}", cache.display()))
        .map_err(stdout_error)?;
    Ok(ExitClass::Success)
}
