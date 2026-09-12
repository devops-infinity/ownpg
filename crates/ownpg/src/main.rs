mod app;
mod build_info;
mod cli;
mod output;

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
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "ownpg: stopped unexpectedly.");
        let _ = writeln!(
            stderr,
            "This is a bug. Please report it with the lines below at"
        );
        let _ = writeln!(stderr, "{}", build_info::ISSUES_URL);
        let _ = writeln!(stderr, "  version: {}", build_info::version_line());
        drop(stderr);
        previous(info);
    }));
}
