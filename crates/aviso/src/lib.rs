//! Core client library for [`aviso-server`], ECMWF's notification service for
//! data-driven workflows.
//!
//! The public surface grows as features land; the design rationale for each
//! choice lives in `docs/src/internals/decisions.md` and is referenced by
//! stable ADR id.
//!
//! [`aviso-server`]: https://github.com/ecmwf/aviso-server

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

mod admin;
pub mod auth;
mod client;
mod error;
pub mod notification;
mod notify;
pub mod schema;
pub mod state;
pub mod watch;

pub use client::{AvisoClient, AvisoClientBuilder};
pub use error::{ClientError, Result};
pub use notification::{Notification, NotificationRequest, NotifyResponse, parse_cloudevent_id};
pub use schema::{SchemaCatalog, SchemaResponse, StreamSchema};

/// Version string of the `aviso` crate, sourced from Cargo metadata.
///
/// Exposed so the CLI and the Python extension can render a single, consistent
/// version line without re-reading `CARGO_PKG_VERSION` themselves.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Process-wide flag controlling whether the built-in echo trigger emits
/// ANSI color escapes around its human-readable output.
///
/// Default: `false` (no color). Library consumers leave this alone unless
/// they specifically want colored echo output; the CLI flips it on at
/// startup when the operator passes `--color auto|always` and the
/// effective color decision (taking the target stream's TTY state and the
/// `NO_COLOR` env var into account) is `true`.
static ECHO_COLOR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables ANSI color codes in the built-in echo trigger's
/// human-readable TTY output. Default: disabled (no color).
///
/// Intended as a CLI-coordination point: the `aviso` binary parses its
/// `--color auto|always|never` flag at startup, computes whether color
/// should fire for the echo trigger (taking `stdout`'s TTY state and the
/// `NO_COLOR` env var into account), and calls this setter ONCE before
/// any listener spawns. Other consumers of the lib (a future Python
/// binding, embedded uses) can ignore this function entirely; the
/// default is off.
///
/// Process-wide global state. Concurrent reads from echo triggers running
/// on multiple tokio workers are thread-safe via `AtomicBool` with
/// `Relaxed` ordering; the value is read once per `dispatch_echo` call,
/// so a late mid-flight change is observed at the next notification
/// rather than mid-format.
pub fn set_echo_color_enabled(enabled: bool) {
    ECHO_COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Returns whether the echo trigger should emit ANSI color codes.
///
/// Crate-internal accessor for [`watch::trigger::echo`]. Public API
/// surface is [`set_echo_color_enabled`] only; downstream consumers
/// who do not call the setter get the default `false`.
pub(crate) fn echo_color_enabled() -> bool {
    ECHO_COLOR_ENABLED.load(Ordering::Relaxed)
}
