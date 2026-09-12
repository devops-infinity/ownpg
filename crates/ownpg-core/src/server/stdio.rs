use std::io::IsTerminal;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Instant;

use rmcp::ServiceExt;
use rmcp::service::QuitReason;
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;

use super::Server;
use crate::error::{Error, ExitClass, Result};

pub const STDIN_LINE_CAP: usize = 16 * 1024 * 1024;
pub const TERMINAL_NOTICE: &str =
    "OwnPG is waiting for an MCP client on stdin; start it from a client, or press Ctrl-C to stop";

pin_project_lite::pin_project! {
    #[derive(Debug)]
    pub struct LineCapReader<R> {
        #[pin]
        inner: R,
        cap: usize,
        current: usize,
    }
}

impl<R> LineCapReader<R> {
    pub const fn new(inner: R, cap: usize) -> Self {
        Self {
            inner,
            cap,
            current: 0,
        }
    }
}

impl<R: AsyncRead> AsyncRead for LineCapReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.project();
        let before = buf.filled().len();
        match this.inner.poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let fresh = buf.filled().get(before..).unwrap_or_default();
                for byte in fresh {
                    if *byte == b'\n' {
                        *this.current = 0;
                    } else {
                        *this.current += 1;
                        if *this.current > *this.cap {
                            return Poll::Ready(Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("a single stdin line exceeded the {} byte cap", this.cap),
                            )));
                        }
                    }
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Eof,
    Terminate,
    Interrupt,
}

async fn wait_for_signal() -> StopReason {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, "SIGTERM cannot be observed");
                let () = std::future::pending().await;
                unreachable!()
            }
        };
        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, "SIGINT cannot be observed");
                let () = std::future::pending().await;
                unreachable!()
            }
        };
        tokio::select! {
            _ = terminate.recv() => StopReason::Terminate,
            _ = interrupt.recv() => StopReason::Interrupt,
        }
    }
    #[cfg(not(unix))]
    {
        match tokio::signal::ctrl_c().await {
            Ok(()) => StopReason::Interrupt,
            Err(error) => {
                tracing::warn!(%error, "Ctrl-C cannot be observed");
                let () = std::future::pending().await;
                unreachable!()
            }
        }
    }
}

pub async fn serve<R, W>(server: Arc<Server>, stdin: R, stdout: W) -> Result<ExitClass>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let capped = LineCapReader::new(stdin, STDIN_LINE_CAP);
    let cancel = CancellationToken::new();
    let running = match Arc::clone(&server)
        .serve_with_ct((capped, stdout), cancel.clone())
        .await
    {
        Ok(running) => running,
        Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => {
            tracing::info!("stdin closed before the first request");
            server.shutdown().await;
            return Ok(ExitClass::Success);
        }
        Err(error) => {
            server.shutdown().await;
            return Err(Error::ProtocolFailed {
                detail: error.to_string(),
            });
        }
    };
    let stop = {
        let mut signal = std::pin::pin!(wait_for_signal());
        let mut waiting = std::pin::pin!(running.waiting());
        tokio::select! {
            quit = &mut waiting => {
                match quit {
                    Ok(QuitReason::Closed) => StopReason::Eof,
                    Ok(QuitReason::Cancelled) => StopReason::Terminate,
                    Ok(_) | Err(_) => StopReason::Eof,
                }
            }
            reason = &mut signal => {
                tracing::info!("stopping; a second signal exits at once");
                cancel.cancel();
                let started = Instant::now();
                let second = std::pin::pin!(wait_for_signal());
                tokio::select! {
                    _ = &mut waiting => {}
                    _ = second => {
                        tracing::warn!("a second signal arrived; exiting at once");
                        return Ok(exit_for(reason));
                    }
                    () = tokio::time::sleep(super::SHUTDOWN_DEADLINE.saturating_sub(started.elapsed())) => {
                        tracing::warn!("the service loop did not stop within the deadline");
                    }
                }
                reason
            }
        }
    };
    let deadline = tokio::time::timeout(
        super::SHUTDOWN_DEADLINE + super::SHUTDOWN_HEADROOM,
        server.shutdown(),
    );
    if deadline.await.is_err() {
        tracing::warn!("shutdown passed its deadline");
    }
    Ok(exit_for(stop))
}

#[must_use]
pub fn exit_for(reason: StopReason) -> ExitClass {
    match reason {
        StopReason::Eof | StopReason::Terminate => ExitClass::Success,
        StopReason::Interrupt => {
            if std::io::stdin().is_terminal() {
                ExitClass::Interrupted
            } else {
                ExitClass::Success
            }
        }
    }
}

#[must_use]
pub fn terminal_notice() -> Option<&'static str> {
    std::io::stdin().is_terminal().then_some(TERMINAL_NOTICE)
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncBufReadExt;

    use super::*;

    #[tokio::test]
    async fn a_line_past_the_cap_is_an_error_and_shorter_lines_pass() {
        let data = b"short\nxxxxxxxxxxxx\n".to_vec();
        let reader = LineCapReader::new(std::io::Cursor::new(data), 8);
        let mut lines = tokio::io::BufReader::with_capacity(4, reader).lines();
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("short"));
        let error = lines.next_line().await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn the_counter_resets_after_every_newline() {
        let data = b"12345678\n12345678\n".to_vec();
        let reader = LineCapReader::new(std::io::Cursor::new(data), 8);
        let mut lines = tokio::io::BufReader::new(reader).lines();
        assert!(lines.next_line().await.unwrap().is_some());
        assert!(lines.next_line().await.unwrap().is_some());
        assert!(lines.next_line().await.unwrap().is_none());
    }
}
