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
    let names_protocol_target = rust_log.is_some_and(|value| value.contains(PROTOCOL_TARGET));
    if !global.quiet
        && !names_protocol_target
        && (global.verbose > 0 || rust_log.is_some())
        && let Ok(parsed) = PROTOCOL_PAYLOAD_CAP.parse()
    {
        env_filter = env_filter.add_directive(parsed);
    }
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

#[derive(Default)]
struct EscapedFields(tracing_subscriber::fmt::format::DefaultFields);

impl<'writer> tracing_subscriber::fmt::FormatFields<'writer> for EscapedFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        mut writer: tracing_subscriber::fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        let mut buffer = String::new();
        self.0.format_fields(
            tracing_subscriber::fmt::format::Writer::new(&mut buffer),
            fields,
        )?;
        std::fmt::Write::write_str(&mut writer, &escape_controls(&buffer))
    }
}

fn escape_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push(c),
            c if c.is_control() => out.push_str(&format!("\\u{{{:04x}}}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn init(global: &GlobalArgs, paths: &AppPaths, human_output: bool) -> Result<LogGuard> {
    let rust_log = std::env::var("RUST_LOG").ok();
    let env_filter = log_filter(global, rust_log.as_deref());
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(io::stderr);
    let plain = human_output && global.verbose == 0 && !global.quiet;
    let stderr_layer = match (global.log_format, plain) {
        (LogFormatArg::Text, true) => stderr_layer
            .fmt_fields(EscapedFields::default())
            .without_time()
            .with_level(false)
            .boxed(),
        (LogFormatArg::Text, false) => stderr_layer.fmt_fields(EscapedFields::default()).boxed(),
        (LogFormatArg::Json, _) => stderr_layer.json().flatten_event(true).boxed(),
    };
    let (file_layer, worker) = match &global.log_file {
        Some(path) => {
            let target = resolve_log_path(path, paths);
            let directory = target.parent().unwrap_or(Path::new("."));
            let (prefix, suffix) = rolled_file_name(&target);
            ownpg_core::config::profile::create_private_directory(directory).map_err(|source| {
                Error::OutputUnwritable {
                    target: directory.display().to_string(),
                    source,
                }
            })?;
            let builder = tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .max_log_files(MAX_LOG_FILES)
                .filename_prefix(prefix);
            let builder = match suffix {
                Some(suffix) => builder.filename_suffix(suffix),
                None => builder,
            };
            let appender = builder
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
                LogFormatArg::Text => layer.fmt_fields(EscapedFields::default()).boxed(),
                LogFormatArg::Json => layer.json().flatten_event(true).boxed(),
            };
            (Some((layer, directory.to_path_buf())), Some(guard))
        }
        None => (None, None),
    };
    let (file_layer, shared_directory) = match file_layer {
        Some((layer, directory)) => (
            Some(layer),
            readable_by_others(&directory).then_some(directory),
        ),
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
    if let Some(directory) = shared_directory {
        tracing::warn!(
            directory = %directory.display(),
            "the log directory is readable by other users, and log files created there inherit that access"
        );
    }
    Ok(LogGuard { _worker: worker })
}

const MAX_LOG_FILES: usize = 8;
const PROTOCOL_TARGET: &str = "rmcp";
const PROTOCOL_PAYLOAD_CAP: &str = "rmcp=info";

fn rolled_file_name(target: &Path) -> (String, Option<String>) {
    let stem = target
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty());
    let extension = target
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .filter(|extension| !extension.is_empty());
    match (stem, extension) {
        (Some(stem), Some(extension)) => (stem, Some(extension)),
        (Some(stem), None) => (stem, None),
        (None, _) => ("ownpg".to_owned(), Some("log".to_owned())),
    }
}

#[cfg(unix)]
fn readable_by_others(directory: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(directory).is_ok_and(|metadata| metadata.permissions().mode() & 0o077 != 0)
}

#[cfg(not(unix))]
fn readable_by_others(_directory: &Path) -> bool {
    false
}

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

    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_line_break_in_a_logged_value_cannot_start_a_fake_log_line() {
        assert_eq!(escape_controls("a\nb\rc\td\u{1b}"), "a\\nb\\rc\td\\u{001b}");
        let captured = Captured::default();
        let writer = captured.clone();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .fmt_fields(EscapedFields::default())
            .with_writer(move || writer.clone());
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                detail = %"one\n2026-09-24T00:00:00Z  INFO forged",
                "a message\nwith a break"
            );
        });
        let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("one\\n2026-09-24"), "{text}");
    }

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
        let trace = log_filter(&global(2, false), None).to_string();
        assert!(trace.contains("trace"), "{trace}");
        assert!(trace.contains("rmcp=info"), "{trace}");
        let debug = log_filter(&global(1, false), None).to_string();
        assert!(debug.contains("rmcp=info"), "{debug}");
        let from_environment = log_filter(&global(0, false), Some("debug")).to_string();
        assert!(from_environment.contains("rmcp=info"), "{from_environment}");
        let composed = log_filter(&global(1, false), Some("rmcp=debug")).to_string();
        assert!(composed.contains("rmcp=debug"));
        assert!(!composed.contains("rmcp=info"), "{composed}");
        assert!(composed.contains("ownpg=debug"));
        let quiet = log_filter(&global(0, true), Some("info")).to_string();
        assert!(quiet.contains("ownpg=error"));
    }

    #[test]
    fn rotated_log_files_keep_the_extension_as_a_suffix() {
        assert_eq!(
            rolled_file_name(Path::new("/var/log/ownpg.log")),
            ("ownpg".to_owned(), Some("log".to_owned()))
        );
        assert_eq!(
            rolled_file_name(Path::new("server")),
            ("server".to_owned(), None)
        );
        assert_eq!(
            rolled_file_name(Path::new("/var/log/.log")),
            (".log".to_owned(), None)
        );
    }

    #[test]
    fn a_bare_log_file_name_lands_under_the_log_directory() {
        let paths = AppPaths::from_base("/c".into(), "/d".into());
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
