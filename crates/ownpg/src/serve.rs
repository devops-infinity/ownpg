use std::io::Write;
use std::sync::Arc;

use ownpg_core::audit::Transport;
use ownpg_core::config::{Sources, resolve};
use ownpg_core::engine::Engine;
use ownpg_core::server::{Principal, Server, audit_sink, http, stdio};
use ownpg_core::{Error, ExitClass, Result};

use crate::cli::{GlobalArgs, ServeArgs};
use crate::context::{self, Process};

pub(crate) fn run(global: &GlobalArgs, args: &ServeArgs, process: &Process) -> Result<ExitClass> {
    let flags = context::flag_layer(&args.connection, global, Some(args))?;
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
    let settings = Arc::new(settings);
    let hints = context::hints(&process.env);
    let principal = Principal::local(process.env.os_user());
    let runtime = if args.http {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    }
    .map_err(|error| Error::ProtocolFailed {
        detail: format!("the runtime could not start: {error}"),
    })?;
    let environment_tokens = process.env.var("OWNPG_BEARER_TOKENS").map(str::to_owned);
    let over_http = args.http;
    runtime.block_on(async move {
        let engine = if over_http {
            Engine::start_pooled(Arc::clone(&settings), hints).await?
        } else {
            Engine::start(Arc::clone(&settings), hints).await?
        };
        let info = engine.info().await;
        if let Some(warning) = info.tls_warning() {
            tracing::warn!("{warning}");
        }
        if let Some(warning) = engine.role().await?.warning() {
            tracing::warn!("{warning}");
        }
        tracing::info!(
            target = %info.target,
            database = %info.database,
            schema = %settings.schema.value,
            mode = %settings.mode.value,
            "connected"
        );
        let sink = Arc::new(audit_sink(&settings)?);
        if over_http {
            let server = Arc::new(
                Server::new(
                    Arc::new(engine),
                    sink,
                    Transport::Http,
                    Principal::local(None),
                )
                .await?,
            );
            let gate = Arc::new(http::gatekeeper(
                Arc::clone(&server),
                &settings,
                environment_tokens.as_deref(),
                Principal::local(None),
            )?);
            let router = http::router(Arc::clone(&gate), &settings);
            let listening = http::Listening::bind(settings.http.bind.value).await?;
            tracing::info!(
                address = %listening.local_addr,
                auth = %settings.http.auth.mode(),
                public_url = %settings.http.public_url.value,
                "OwnPG is serving Streamable HTTP"
            );
            return http::serve(
                listening,
                router,
                gate,
                http::shutdown_signal(),
                settings.http.shutdown.value,
            )
            .await;
        }
        let server = Server::new(Arc::new(engine), sink, Transport::Stdio, principal).await?;
        if let Some(notice) = stdio::terminal_notice() {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "{notice}");
        }
        stdio::serve(Arc::new(server), tokio::io::stdin(), tokio::io::stdout()).await
    })
}
