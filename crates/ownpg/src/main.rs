mod app;
mod audit_cmd;
mod build_info;
mod cli;
mod config_cmd;
mod context;
mod doctor;
mod health;
mod logging;
mod man;
mod output;
mod serve;

use std::io::{self, Write};
use std::process::ExitCode;

use crate::cli::Cli;

fn main() -> ExitCode {
    install_panic_hook();
    let args = Cli::parse_args(build_info::version_line());
    ExitCode::from(app::run(args).code())
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let with_backtrace = backtrace_requested();
        {
            let mut stderr = io::stderr().lock();
            write_panic_report(&mut stderr, info, with_backtrace);
        }
        if with_backtrace {
            previous(info);
        }
    }));
}

fn write_panic_report(
    out: &mut impl Write,
    info: &std::panic::PanicHookInfo<'_>,
    with_backtrace: bool,
) {
    let _ = writeln!(out, "OwnPG stopped unexpectedly.");
    let _ = writeln!(
        out,
        "This is a bug. Please report it with the lines below at"
    );
    let _ = writeln!(out, "{}", build_info::ISSUES_URL);
    let _ = writeln!(out, "  version: {}", build_info::version_line());
    if !with_backtrace {
        let _ = writeln!(out, "{info}");
    }
}

fn backtrace_requested() -> bool {
    std::env::var("RUST_BACKTRACE").is_ok_and(|value| value != "0")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn the_report_names_the_panic_location_and_message_without_a_backtrace_flag() {
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_hook = Arc::clone(&captured);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Ok(mut buffer) = captured_for_hook.lock() {
                write_panic_report(&mut *buffer, info, false);
            }
        }));
        let result = std::panic::catch_unwind(|| panic!("a deliberate test panic"));
        std::panic::set_hook(previous);
        assert!(result.is_err());
        let text = String::from_utf8(captured.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .unwrap_or_default();
        assert!(text.contains("OwnPG stopped unexpectedly."), "{text}");
        assert!(text.contains("a deliberate test panic"), "{text}");
        assert!(text.contains(file!()), "{text}");
    }

    #[test]
    fn the_report_omits_the_raw_panic_line_when_a_backtrace_will_follow() {
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_hook = Arc::clone(&captured);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Ok(mut buffer) = captured_for_hook.lock() {
                write_panic_report(&mut *buffer, info, true);
            }
        }));
        let result = std::panic::catch_unwind(|| panic!("a second deliberate test panic"));
        std::panic::set_hook(previous);
        assert!(result.is_err());
        let text = String::from_utf8(captured.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .unwrap_or_default();
        assert!(text.contains("OwnPG stopped unexpectedly."), "{text}");
        assert!(!text.contains("panicked at"), "{text}");
    }
}
