// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Core client library for [`aviso-server`], ECMWF's notification service for
//! data-driven workflows.
//!
//! The public surface grows as features land; the design rationale for each
//! choice lives in `plans/decisions.md` and is referenced by stable ADR id.
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

pub use client::{AvisoClient, AvisoClientBuilder, ClientSettings};
pub use error::{ClientError, Result};
pub use notification::{Notification, NotificationRequest, NotifyResponse, parse_cloudevent_id};
pub use notify::DEFAULT_NOTIFY_CONCURRENCY;
pub use schema::{SchemaCatalog, SchemaResponse, StreamSchema};

/// Version string of the `aviso` crate, sourced from Cargo metadata.
///
/// Exposed so the CLI and the Python extension can render a single, consistent
/// version line without re-reading `CARGO_PKG_VERSION` themselves.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Process-wide flag controlling whether the built-in echo trigger emits
/// ANSI color escapes around its human-readable output.
///
/// Default: `false` (no color). The flag is owned by the host application
/// (the `aviso` CLI binary, a future `PyO3` binding, an embedding Rust
/// application) and set via [`set_echo_color_enabled`] before listeners
/// spawn; the lib itself never decides color policy.
static ECHO_COLOR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Sets the host-application-level policy for ANSI color codes in the
/// built-in echo trigger's human-readable TTY output. Default: disabled
/// (no color).
///
/// Echo lives in the library because it is a built-in trigger like
/// webhook, log, command, teams, and post; library consumers need it
/// even when they do not run on top of `aviso-cli`. TTY-state detection
/// and `NO_COLOR`-handling are host concerns (a daemon embedding the lib
/// has different rules from an interactive CLI) and the lib refuses to
/// guess. This setter is the narrow public surface through which the
/// host communicates its resolved decision to the in-lib trigger:
///
/// - The `aviso` CLI binary parses `--color auto|always|never`,
///   inspects `stdout`'s TTY state, honors `NO_COLOR`, and calls this
///   ONCE at startup before any listener spawns.
/// - A future `PyO3` binding can mirror that pattern from Python.
/// - An embedded Rust application that does not want color simply
///   never calls the setter; the default is off.
///
/// Process-wide global state. Concurrent reads from echo triggers
/// running on multiple tokio workers are thread-safe via `AtomicBool`
/// with `Relaxed` ordering; the value is read once per `dispatch_echo`
/// call, so a late mid-flight change is observed at the next
/// notification rather than mid-format. The flag does not synchronize
/// any other state, which is why `Relaxed` is sound.
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
