use std::io::{IsTerminal, Read};

use ownpg_core::config::describe::describe;
use ownpg_core::config::profile::{ProfileFile, write_private};
use ownpg_core::config::{Sources, resolve};
use ownpg_core::{Error, ExitClass, Result};

use crate::cli::{ConfigCommand, GlobalArgs, OutputFormatArg};
use crate::context::{self, KEYCHAIN_SERVICE, Process, keychain_account};
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
                    let rendered = serde_json::to_string_pretty(&lines).map_err(|error| {
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
        ConfigCommand::Path => {
            let paths = &process.paths;
            emit(|out| {
                writeln!(out, "profiles: {}", paths.config_file.display())?;
                writeln!(out, "data: {}", paths.data_dir.display())?;
                writeln!(out, "cache: {}", paths.cache_dir.display())
            })
            .map_err(stdout_error)?;
            Ok(ExitClass::Success)
        }
        ConfigCommand::Init { force, dry_run } => init(process, *force, *dry_run),
        ConfigCommand::SetPassword { profile } => set_password(global, process, profile),
        ConfigCommand::UnsetPassword { profile } => unset_password(process, profile),
        ConfigCommand::CacheClear => cache_clear(process),
    }
}

fn init(process: &Process, force: bool, dry_run: bool) -> Result<ExitClass> {
    let path = &process.paths.config_file;
    let example = ProfileFile::example();
    let text = toml::to_string_pretty(&example).map_err(|error| Error::ConfigUnwritable {
        path: path.clone(),
        source: std::io::Error::other(error.to_string()),
    })?;
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

fn read_password(global: &GlobalArgs, process: &Process) -> Result<String> {
    let no_input = global.no_input || process.env.var("CI").is_some_and(|value| value == "true");
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        if no_input {
            return Err(Error::ArgumentInvalid {
                argument: "--no-input".to_owned(),
                detail: "a password prompt is needed; pipe the password on stdin instead"
                    .to_owned(),
            });
        }
        return rpassword::prompt_password("Password (not echoed): ").map_err(|source| {
            Error::OutputUnwritable {
                target: "the terminal".to_owned(),
                source,
            }
        });
    }
    let mut text = String::new();
    stdin
        .lock()
        .read_to_string(&mut text)
        .map_err(|source| Error::OutputUnwritable {
            target: "stdin".to_owned(),
            source,
        })?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn keychain_entry(profile: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &keychain_account(profile)).map_err(|error| {
        Error::ConfigInvalid {
            setting: "password_keychain".to_owned(),
            value: profile.to_owned(),
            detail: error.to_string(),
        }
    })
}

fn set_password(global: &GlobalArgs, process: &Process, profile: &str) -> Result<ExitClass> {
    let mut file = load_profiles(process, profile)?;
    let password = read_password(global, process)?;
    if password.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: "password".to_owned(),
            detail: "an empty password was given".to_owned(),
        });
    }
    keychain_entry(profile)?
        .set_password(&password)
        .map_err(|error| Error::ConfigInvalid {
            setting: "password_keychain".to_owned(),
            value: profile.to_owned(),
            detail: error.to_string(),
        })?;
    if let Some(entry) = file.profiles.get_mut(profile) {
        entry.password = None;
        entry.password_keychain = Some(true);
    }
    file.save(&process.paths.config_file)?;
    tracing::info!(profile, "password stored in the platform keychain");
    Ok(ExitClass::Success)
}

fn unset_password(process: &Process, profile: &str) -> Result<ExitClass> {
    let mut file = load_profiles(process, profile)?;
    match keychain_entry(profile)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => {
            return Err(Error::ConfigInvalid {
                setting: "password_keychain".to_owned(),
                value: profile.to_owned(),
                detail: error.to_string(),
            });
        }
    }
    if let Some(entry) = file.profiles.get_mut(profile) {
        entry.password_keychain = None;
    }
    file.save(&process.paths.config_file)?;
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
