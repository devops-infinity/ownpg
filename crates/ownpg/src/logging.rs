use std::io;
use std::path::Path;

use ownpg_core::config::AppPaths;
use ownpg_core::{Error, Result};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::cli::{GlobalArgs, LogFormatArg};

#[derive(Debug)]
pub(crate) struct LogGuard {
    _worker: Option<tracing_appender::non_blocking::WorkerGuard>,
}

pub(crate) fn log_filter(global: &GlobalArgs, rust_log: Option<&str>) -> EnvFilter {
    let base = match (global.quiet, global.verbose) {
        (true, _) => "error",
        (false, 0) => "warn,ownpg=info,ownpg_core=info,rmcp=warn",
        (false, 1) => "info,ownpg=debug,ownpg_core=debug",
        (false, _) => "trace",
    };
    let mut env_filter = rust_log.map_or_else(
        || EnvFilter::new(base),
        |value| EnvFilter::builder().parse_lossy(value),
    );
    if global.quiet {
        for directive in ["ownpg=error", "ownpg_core=error"] {
            if let Ok(parsed) = directive.parse() {
                env_filter = env_filter.add_directive(parsed);
            }
        }
    } else if global.verbose > 0 && rust_log.is_some() {
        let level = if global.verbose == 1 {
            "debug"
        } else {
            "trace"
        };
        for target in ["ownpg", "ownpg_core"] {
            if let Ok(parsed) = format!("{target}={level}").parse() {
                env_filter = env_filter.add_directive(parsed);
            }
        }
    }
    env_filter
}

pub(crate) fn init(global: &GlobalArgs, paths: &AppPaths, human: bool) -> Result<LogGuard> {
    let rust_log = std::env::var("RUST_LOG").ok();
    let env_filter = log_filter(global, rust_log.as_deref());
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(io::stderr);
    let plain = human && global.verbose == 0 && !global.quiet;
    let stderr_layer = match (global.log_format, plain) {
        (LogFormatArg::Text, true) => stderr_layer.without_time().with_level(false).boxed(),
        (LogFormatArg::Text, false) => stderr_layer.boxed(),
        (LogFormatArg::Json, _) => stderr_layer.json().flatten_event(true).boxed(),
    };
    let (file_layer, worker) = match &global.log_file {
        Some(path) => {
            let target = resolve_log_path(path, paths);
            let directory = target.parent().unwrap_or(Path::new("."));
            let file_name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "ownpg.log".to_owned());
            ownpg_core::config::profile::create_private_directory(directory).map_err(|source| {
                Error::OutputUnwritable {
                    target: directory.display().to_string(),
                    source,
                }
            })?;
            let appender = tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .max_log_files(KEPT_LOG_FILES)
                .filename_prefix(file_name)
                .build(directory)
                .map_err(|error| Error::OutputUnwritable {
                    target: directory.display().to_string(),
                    source: std::io::Error::other(error.to_string()),
                })?;
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            let layer = match global.log_format {
                LogFormatArg::Text => layer.boxed(),
                LogFormatArg::Json => layer.json().flatten_event(true).boxed(),
            };
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .try_init()
        .map_err(|error| Error::ProtocolFailed {
            detail: format!("the log subscriber could not start: {error}"),
        })?;
    Ok(LogGuard { _worker: worker })
}

const KEPT_LOG_FILES: usize = 8;

pub(crate) fn resolve_log_path(path: &Path, paths: &AppPaths) -> std::path::PathBuf {
    if path.components().count() > 1 || path.is_absolute() {
        path.to_path_buf()
    } else {
        paths.log_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(verbose: u8, quiet: bool) -> GlobalArgs {
        GlobalArgs {
            verbose,
            quiet,
            no_input: false,
            log_format: LogFormatArg::Text,
            log_file: None,
            config: None,
        }
    }

    #[test]
    fn verbosity_and_rust_log_compose() {
        let default = log_filter(&global(0, false), None).to_string();
        for directive in ["warn", "ownpg=info", "ownpg_core=info", "rmcp=warn"] {
            assert!(default.contains(directive), "{default}");
        }
        assert_eq!(log_filter(&global(2, false), None).to_string(), "trace");
        let composed = log_filter(&global(1, false), Some("rmcp=debug")).to_string();
        assert!(composed.contains("rmcp=debug"));
        assert!(composed.contains("ownpg=debug"));
        let quiet = log_filter(&global(0, true), Some("info")).to_string();
        assert!(quiet.contains("ownpg=error"));
    }

    #[test]
    fn a_bare_log_file_name_lands_under_the_log_directory() {
        let paths = AppPaths::from_base("/c".into(), "/d".into(), "/k".into());
        assert_eq!(
            resolve_log_path(Path::new("ownpg.log"), &paths),
            Path::new("/d/logs/ownpg.log")
        );
        let custom = paths.clone().with_log_dir("/var/log/ownpg".into());
        assert_eq!(
            resolve_log_path(Path::new("ownpg.log"), &custom),
            Path::new("/var/log/ownpg/ownpg.log")
        );
        assert_eq!(
            resolve_log_path(Path::new("/var/log/ownpg.log"), &paths),
            Path::new("/var/log/ownpg.log")
        );
        assert_eq!(
            resolve_log_path(Path::new("logs/ownpg.log"), &paths),
            Path::new("logs/ownpg.log")
        );
    }
}
