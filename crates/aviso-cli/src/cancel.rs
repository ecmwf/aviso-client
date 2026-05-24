//! Ctrl+C handler for graceful shutdown plus a hard-exit second-SIGINT escape.
//!
//! [`install`] spawns a dedicated tokio task that awaits
//! [`tokio::signal::ctrl_c`] in a loop. The FIRST signal flips the
//! returned [`tokio::sync::watch::Sender<bool>`] to `true`; subcommand
//! handlers select on `.changed()` and drain gracefully (the
//! supervisor's existing drop cascade per D19 / D20 closes the watch
//! and lands pending commits). A SECOND signal received within
//! 5 seconds of the first calls [`std::process::exit`] with code
//! `130` (`128 + SIGINT`) immediately, bypassing the cooperative
//! drain.
//!
//! The 5-second window is intentional: it gives the supervisor time
//! to land a pending state-store commit but bounds the operator's
//! wait if something is stuck. The window is measured from the
//! first signal's instant via a [`std::sync::Mutex`] guarding an
//! `Option<Instant>`; subsequent signals compare to that marker.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::watch;

/// Maximum window between first and second SIGINT after which the
/// second signal also triggers graceful behaviour rather than the
/// hard exit. Long enough to cover state-store commit latency on
/// slow disks (NFS, USB drives), short enough not to surprise an
/// operator hitting Ctrl+C twice on purpose.
const DOUBLE_SIGINT_WINDOW: Duration = Duration::from_secs(5);

/// Installs the Ctrl+C handler and returns the cancellation
/// [`watch::Receiver`] subcommand handlers can `.select!` on.
///
/// The returned receiver fires `cancel.changed().await` when the
/// first SIGINT arrives. The handler task itself stays running
/// until the process exits; it owns the `Sender` half so the
/// receiver does not see `Lagged`.
///
/// Failures to install the signal handler are logged at `WARN`
/// from within the spawned task (the lib does not crash on signal-
/// API failure; the operator just loses the graceful-shutdown
/// affordance and must rely on the OS sending SIGKILL).
pub(crate) fn install() -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    let last_signal: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    tokio::spawn(handler_task(sender, last_signal));
    receiver
}

async fn handler_task(sender: watch::Sender<bool>, last_signal: Arc<Mutex<Option<Instant>>>) {
    loop {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                let now = Instant::now();
                let mut guard = match last_signal.lock() {
                    Ok(g) => g,
                    Err(poison) => poison.into_inner(),
                };
                let window_secs = DOUBLE_SIGINT_WINDOW.as_secs();
                if let Some(prev) = *guard {
                    if now.duration_since(prev) <= DOUBLE_SIGINT_WINDOW {
                        // Variable-capture form: `window_secs` is both a
                        // structured field and a format-arg, position-
                        // independent. The `field = expr` form also works
                        // but only AFTER the message string, which is
                        // brittle under future reorders.
                        tracing::warn!(
                            event.name = "cli.sigint.hard_exit",
                            window_secs,
                            "second SIGINT received within {window_secs}s; hard-exiting with code 130",
                        );
                        std::process::exit(crate::exit::HARD_SIGINT);
                    }
                }
                *guard = Some(now);
                drop(guard);
                tracing::debug!(
                    event.name = "cli.sigint.received",
                    "SIGINT received; draining gracefully",
                );
                let _ = crate::output::write_stderr_line(&format!(
                    "Stopping listeners gracefully (Ctrl+C again within {window_secs}s to force exit)..."
                ));
                if sender.send(true).is_err() {
                    return;
                }
            }
            Err(e) => {
                tracing::warn!(
                    event.name = "cli.sigint.install_failed",
                    error = %e,
                    "could not install Ctrl+C handler; graceful shutdown unavailable"
                );
                // Park the handler task forever so the watch::Sender stays
                // alive. Dropping the Sender would close the channel and
                // downstream `.changed().await` would resolve as a
                // cancellation signal, causing every listen / replay
                // subcommand to exit immediately on platforms where signal
                // installation fails (Windows without a console, restricted
                // sandboxes, no-TTY container init). Operators lose graceful
                // shutdown there but the workload itself keeps running; OS-
                // level SIGKILL remains the fallback stop mechanism.
                std::future::pending::<()>().await;
                unreachable!("std::future::pending never resolves");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_sigint_window_is_five_seconds() {
        assert_eq!(DOUBLE_SIGINT_WINDOW, Duration::from_secs(5));
    }
}
