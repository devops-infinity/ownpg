use std::time::Duration;

use ownpg_core::config::{FlagLayer, HttpFlags, Sources, resolve_bind};
use ownpg_core::server::http::{LIVE_PATH, READY_PATH, probe};
use ownpg_core::{Error, ExitClass, Result};

use crate::cli::HealthArgs;
use crate::context::Process;
use crate::output::{emit, report_error, stdout_error};

pub(crate) fn run(args: &HealthArgs, process: &Process) -> Result<ExitClass> {
    match check(args, process) {
        Ok(answer) => {
            emit(|out| writeln!(out, "{answer}")).map_err(stdout_error)?;
            Ok(ExitClass::Success)
        }
        Err(error) => {
            report_error(&error);
            Ok(ExitClass::Runtime)
        }
    }
}

fn check(args: &HealthArgs, process: &Process) -> Result<String> {
    let flags = FlagLayer {
        http: HttpFlags {
            enabled: true,
            bind: args.bind.clone(),
            auth: None,
        },
        ..FlagLayer::default()
    };
    let bind = resolve_bind(
        &flags,
        &Sources {
            env: &process.env,
            paths: process.paths.clone(),
            keychain: None,
        },
    )?;
    let path = if args.live { LIVE_PATH } else { READY_PATH };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::ProtocolFailed {
            detail: format!("the runtime could not start: {error}"),
        })?;
    runtime.block_on(probe(bind, path, Duration::from_secs(args.timeout)))
}
