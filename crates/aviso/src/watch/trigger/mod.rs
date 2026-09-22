// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Trigger configuration types and dispatcher for watch sessions.
//!
//! A [`Trigger`] is a per-notification side effect attached to a
//! [`crate::watch::WatchRequest`] via
//! [`crate::watch::WatchRequest::with_triggers`]. Six built-in kinds
//! ship in the core:
//!
//! - [`Trigger::echo`]: writes the notification as NDJSON to standard
//!   output (with a pretty-JSON variant on TTY).
//! - [`Trigger::log`]: appends the notification as NDJSON to a
//!   user-specified file.
//! - [`Trigger::command`]: runs `/bin/sh -c <rendered>` per notification
//!   with the notification's fields exposed as `AVISO_*` environment
//!   variables. Unix-only.
//! - [`Trigger::webhook`]: sends an HTTP request per notification to a
//!   user-configured URL, with the URL, header values, and body all
//!   template-rendered. Cross-platform.
//! - [`Trigger::teams`]: posts an auto-built Adaptive Card to a
//!   Microsoft Teams Workflows webhook URL. Delegates the HTTP send
//!   to the webhook machinery so retry classification, response
//!   capture, and error mapping are shared.
//! - [`Trigger::post`]: HTTP POST that forwards the raw server-emitted
//!   `CloudEvent` envelope; falls back to a reconstructed envelope only
//!   when [`crate::Notification::cloudevent`] is `None` (test fixtures).
//!
//! Each trigger has a `retries` count (default `0`), a `required` flag
//! (default `true`), a `timeout` (default `None` for echo, log, and
//! command; [`DEFAULT_WEBHOOK_TIMEOUT`] for the HTTP-based triggers
//! webhook, teams, and post), and a `fail_fast` flag (default `true`;
//! meaningful for the command trigger and the HTTP-based triggers).
//! A required trigger that fails after all retries terminates the
//! watch with [`crate::ClientError::TriggerFailed`]; an optional
//! trigger that fails logs a `WARN` event and the watch continues.
//!
//! # Dispatcher contract
//!
//! [`dispatch_triggers`] is the supervisor's entry point. It runs each
//! configured trigger sequentially in declaration order, with the
//! per-trigger `retries` budget and the supervisor's `compute_backoff`
//! schedule between attempts. A single dispatch attempt runs to completion (it is the
//! atomic unit; the dispatcher does NOT race the attempt against
//! cancellation). Between attempts and between triggers, the dispatcher
//! honours both `parent_cancel` and the per-stream `cancel` oneshot.
//!
//! The error surface returned by trigger dispatch is [`TriggerError`]; the
//! human-readable kind tag that appears on
//! `ClientError::TriggerFailed` is [`TriggerKindLabel`].

use std::path::PathBuf;

use kind::TriggerKind;

pub(crate) use dispatcher::dispatch_triggers;
pub use error::{TriggerError, TriggerKindLabel};
pub use http_method::HttpMethod;
pub use template::TemplateErrorKind;
#[cfg(unix)]
pub use yaml::CommandTriggerConfig;
pub use yaml::{EchoConfig, LogConfig, TriggerConfig, WebhookTriggerConfig};

/// Command trigger dispatch. Unix-only (`#[cfg(unix)]`).
#[cfg(unix)]
mod command;
/// Trigger dispatch orchestration.
mod dispatcher;
/// Echo trigger dispatch.
mod echo;
/// Public error and label types for the trigger surface.
mod error;
/// HTTP method enum used by the webhook trigger.
mod http_method;
/// Trigger kind internals.
mod kind;
/// Log trigger dispatch.
mod log;
/// Post trigger dispatch (HTTP POST with CloudEvent-shaped body).
mod post;
/// Teams trigger dispatch (sugar over webhook with auto-built Adaptive Card body).
mod teams;
/// Template substitution engine shared by the command, webhook,
/// teams, and post triggers.
mod template;
/// Webhook trigger dispatch.
mod webhook;
/// Declarative YAML configuration for triggers.
mod yaml;

#[cfg(test)]
mod tests;

pub use webhook::DEFAULT_WEBHOOK_TIMEOUT;

/// A single trigger configured on a watch.
///
/// Built via [`Self::echo`], [`Self::log`], [`Self::command`],
/// [`Self::webhook`], [`Self::teams`], or [`Self::post`]; tuned
/// via chainable setters ([`Self::retries`], [`Self::required`],
/// [`Self::timeout`], [`Self::fail_fast`], [`Self::env`],
/// [`Self::working_dir`]; the last two apply only to command).
/// Defaults are at-least-once safe: required, zero retries,
/// no timeout (HTTP-based triggers seed [`DEFAULT_WEBHOOK_TIMEOUT`]
/// instead), fail-fast on.
///
/// # Examples
///
/// ```
/// use aviso::watch::{Trigger, WatchRequest};
///
/// // Default echo trigger: required, no retries.
/// let echo = Trigger::echo();
///
/// // Log trigger that retries twice on transient I/O failure and is
/// // optional (failure WARNs and the watch continues).
/// let log = Trigger::log("/var/log/aviso/notifications.log")
///     .retries(2)
///     .required(false);
///
/// let req = WatchRequest::watch("mars").with_triggers(vec![echo, log]);
/// assert_eq!(req.triggers().len(), 2);
/// ```
#[non_exhaustive]
#[derive(Clone)]
pub struct Trigger {
    kind: TriggerKind,
    pub(crate) retries: u32,
    pub(crate) required: bool,
    pub(crate) timeout: Option<std::time::Duration>,
    pub(crate) fail_fast: bool,
}

/// Manual `Debug` impl rather than `#[derive(Debug)]`: the derived form
/// does not count as a "production read" for rustc's `dead_code` lint,
/// so a private field that is only constructed but never read elsewhere
/// would still warn even with the derive. The destructure-then-name-each-
/// field pattern below is what the lint counts as a read.
impl std::fmt::Debug for Trigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Trigger {
            kind,
            retries,
            required,
            timeout,
            fail_fast,
        } = self;
        f.debug_struct("Trigger")
            .field("kind", kind)
            .field("retries", retries)
            .field("required", required)
            .field("timeout", timeout)
            .field("fail_fast", fail_fast)
            .finish()
    }
}

impl Trigger {
    /// Build an echo trigger that writes each notification as a single
    /// line of compact JSON to standard output.
    #[must_use]
    pub fn echo() -> Self {
        Self {
            kind: TriggerKind::Echo { label: None },
            retries: 0,
            required: true,
            timeout: None,
            fail_fast: true,
        }
    }

    /// Attach a listener-attribution label to an echo trigger. When the
    /// label is present, the TTY echo dispatch prepends
    /// `(listener: <label>, trigger: echo)` to the leader line so
    /// multi-listener configurations identify which listener produced
    /// each delivery. No-op for non-echo trigger kinds.
    #[must_use]
    pub fn label(mut self, name: impl Into<String>) -> Self {
        if let TriggerKind::Echo { label } = &mut self.kind {
            *label = Some(name.into());
        }
        self
    }

    /// Build a log trigger that appends each notification as a single
    /// line of compact JSON to the file at `path`. The file is opened
    /// with `append(true).create(true)` on first dispatch and held
    /// open for the trigger's lifetime; no rotation, no per-write
    /// `fsync`.
    #[must_use]
    pub fn log(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: TriggerKind::Log { path: path.into() },
            retries: 0,
            required: true,
            timeout: None,
            fail_fast: true,
        }
    }

    /// Build a command trigger that runs `/bin/sh -c <cmd>` once per
    /// notification, with the notification's fields exposed as
    /// `AVISO_*` environment variables. The command string is rendered
    /// through the trigger template engine: `{{ notification.<path> }}`
    /// substitutes a notification field, `{{ env.<NAME> }}` reads from
    /// the process environment.
    ///
    /// # Notification values are data, never code
    ///
    /// Identifier and payload values are written by whoever published
    /// the notification. Each substituted notification value is quoted
    /// for the shell context it lands in, so `run {{ notification.x }}`,
    /// `run '{{ notification.x }}'` and `run "{{ notification.x }}"` all
    /// pass the value to `run` as one literal argument, whatever it
    /// contains. A value cannot add a command, redirect output, or
    /// expand `$(...)`. This covers a placeholder that is part of a
    /// command word. It does not cover backticks, `$( )` inside double
    /// quotes, here-document bodies, or text passed to `eval`; use the
    /// `AVISO_*` environment variables there. `{{ env.* }}` values are
    /// the operator's own and are inserted as written.
    ///
    /// # Environment variable injection
    ///
    /// The dispatcher injects `AVISO_EVENT_TYPE`, `AVISO_SEQUENCE`,
    /// `AVISO_IDENTIFIER_<KEY>` per identifier entry (uppercased,
    /// non-alphanumerics replaced with `_`), `AVISO_PAYLOAD_JSON`
    /// (full payload as compact JSON), and `AVISO_NOTIFICATION_JSON`
    /// (full notification as compact JSON). User-supplied env vars
    /// via [`Self::env`] are applied
    /// AFTER the dispatcher-injected vars, so user keys override
    /// dispatcher keys when both are present.
    ///
    /// The child does not inherit `AVISO_TOKEN`, `AVISO_USERNAME` or
    /// `AVISO_PASSWORD`, nor any variable the command string reads
    /// through `{{ env.NAME }}`. To hand one of these to the child on
    /// purpose, set it through [`Self::env`].
    ///
    /// # Output capture
    ///
    /// Stdout and stderr are captured concurrently into 4 KiB ring
    /// buffers. Stdout content is dropped per the no-payload-logging
    /// discipline; only the captured byte count reaches DEBUG-level
    /// tracing. Stderr tail surfaces in the public
    /// [`crate::ClientError::TriggerFailed`] variant on non-zero exit.
    ///
    /// # Shell descendant cleanup, Unix-only, and template errors
    ///
    /// The dispatcher kills the `/bin/sh -c ...` child when the
    /// [`Self::timeout`] expires, but does NOT propagate the kill
    /// signal to pipelines, backgrounded jobs, or grandchildren that
    /// survive the shell. Use `exec ./binary` so the shell PID
    /// equals the target binary's PID if you need full process-tree
    /// cleanup. The method and the related command-trigger surface
    /// ([`TriggerKindLabel::Command`], [`TriggerError::Command`],
    /// [`Self::env`], [`Self::working_dir`]) are `#[cfg(unix)]`;
    /// Windows builds compile cleanly without them. The constructor
    /// is infallible; a malformed template surfaces at first
    /// dispatch as [`TriggerError::Template`]. See the [`Trigger`]
    /// struct doc for tunable defaults.
    #[cfg(unix)]
    #[must_use]
    pub fn command(cmd: impl Into<String>) -> Self {
        Self {
            kind: TriggerKind::Command(Box::new(command::build_command_config(cmd))),
            retries: 0,
            required: true,
            timeout: None,
            fail_fast: true,
        }
    }

    /// Adds an environment variable to the command trigger's child
    /// process. Repeatable; later sets override earlier ones with
    /// the same key. Values are passed LITERALLY to the child
    /// (they are NOT template-rendered); only the `command:` string
    /// itself goes through the template engine. Silently ignored
    /// on non-command triggers. Unix-only (`#[cfg(unix)]`).
    #[cfg(unix)]
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if let TriggerKind::Command(cfg) = &mut self.kind {
            cfg.env.insert(key.into(), value.into());
        }
        self
    }

    /// Sets the working directory for the command trigger's child
    /// process. If the path does not exist or is not a directory,
    /// dispatch returns [`TriggerError::Io`] at first invocation.
    /// Silently ignored on non-command triggers.
    /// Unix-only (`#[cfg(unix)]`).
    #[cfg(unix)]
    #[must_use]
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        if let TriggerKind::Command(cfg) = &mut self.kind {
            cfg.working_dir = Some(dir.into());
        }
        self
    }

    /// Override the retry count.
    ///
    /// `retries: N` means up to `N` additional attempts after the first
    /// failure, for a total of `N + 1` attempts. Between attempts the
    /// supervisor sleeps using its standard exponential backoff with full
    /// jitter.
    #[must_use]
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Override the required flag.
    ///
    /// A required trigger (default) that fails after all retries terminates
    /// the watch with [`crate::ClientError::TriggerFailed`]. An optional trigger logs
    /// a `WARN` event with stable name `client.trigger.failed` and the watch
    /// continues.
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set a per-trigger timeout.
    ///
    /// Meaningful for the command trigger and the HTTP-based
    /// triggers (webhook, teams, post). For command: bounds the
    /// wait on the child process with `tokio::time::sleep` raced
    /// against `child.wait()`; on expiry the dispatcher issues
    /// `SIGKILL`, reaps the zombie, and returns
    /// [`TriggerError::Timeout`]. For HTTP-based triggers: applied
    /// as `reqwest::RequestBuilder::timeout(t)`; on expiry reqwest
    /// drops the in-flight request and the dispatcher returns
    /// [`TriggerError::Timeout`]. The webhook, teams, and post
    /// constructors all seed the timeout to
    /// [`DEFAULT_WEBHOOK_TIMEOUT`] (30s); calling `.timeout(d)`
    /// overrides it.
    ///
    /// Silently ignored on echo or log triggers: each builds a
    /// buffer (the JSON body with the newline appended to the same
    /// `Vec<u8>`) and submits it via one `write_all` call. The
    /// `write_all` itself may internally loop on short writes (it
    /// is not promised as one `write(2)` syscall), but the
    /// dispatcher does not race the call against cancellation, so
    /// a notification's NDJSON line is never truncated mid-write
    /// by the cancellation `select!`.
    #[must_use]
    pub fn timeout(mut self, t: std::time::Duration) -> Self {
        self.timeout = Some(t);
        self
    }

    /// Override the fail-fast policy on terminal failures.
    ///
    /// When `true` (the default), terminal failures bypass the retry
    /// budget and the trigger fails immediately. When `false`, every
    /// failure is treated as retryable up to the configured
    /// [`Self::retries`] budget.
    ///
    /// Meaningful for the command trigger and the HTTP-based
    /// triggers (webhook, teams, post).
    /// [`TriggerError::Command`] (non-zero exit),
    /// [`TriggerError::Webhook`] with a 4xx status,
    /// [`TriggerError::WebhookBuild`] (HTTP client rejected the
    /// rendered request at build time: malformed URL, invalid
    /// header value), and every [`TriggerError::Template`] (any
    /// render-time failure: missing notification path, missing env
    /// var, malformed template) are terminal under `fail_fast =
    /// true` because they are deterministic with respect to the
    /// current notification and process environment (the same
    /// input produces the same failure).
    /// [`TriggerError::Webhook`] with a 5xx status or `None` status
    /// (transport error), `Io`, `Encode`, and `Timeout` stay
    /// retryable because they are genuinely transient.
    ///
    /// Has no effect on echo or log triggers (their errors are
    /// always retryable through the normal retry budget).
    #[must_use]
    pub fn fail_fast(mut self, on: bool) -> Self {
        self.fail_fast = on;
        self
    }
}

#[cfg(test)]
impl Trigger {
    /// Test-only constructor for a trigger backed by
    /// [`TriggerKind::TestFailing`]. Returns the trigger together with the
    /// shared atomic counter so the test can inspect the remaining-failures
    /// state after dispatch.
    fn test_failing(
        failures_remaining: u32,
        eventual: kind::TestEventual,
        retries: u32,
        required: bool,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicU32>) {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(failures_remaining));
        let trigger = Self {
            kind: TriggerKind::TestFailing {
                failures_remaining: counter.clone(),
                eventual,
            },
            retries,
            required,
            timeout: None,
            fail_fast: true,
        };
        (trigger, counter)
    }

    /// Test-only constructor for a trigger backed by
    /// [`TriggerKind::TestFailOnCall`]. Returns the trigger together with
    /// the shared atomic counter so the test can inspect the per-call
    /// count after dispatch.
    pub(crate) fn test_fail_on_call(
        fail_on_call: u32,
        retries: u32,
        required: bool,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicU32>) {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let trigger = Self {
            kind: TriggerKind::TestFailOnCall {
                calls: counter.clone(),
                fail_on_call,
            },
            retries,
            required,
            timeout: None,
            fail_fast: true,
        };
        (trigger, counter)
    }
}

/// Per-watch, per-trigger mutable state held by the dispatcher across
/// notifications. The log file handle is opened lazily on first dispatch
/// and held for the trigger's lifetime; dropped on supervisor exit (which
/// closes the file via tokio's `File` drop).
pub(super) struct TriggerState {
    pub(super) log_handle: Option<tokio::fs::File>,
}

impl TriggerState {
    pub(super) fn new() -> Self {
        Self { log_handle: None }
    }
}

/// Reason the trigger pipeline aborted early.
///
/// The supervisor maps `RequiredFailed` to
/// [`crate::ClientError::TriggerFailed`] and surfaces it as the watch's
/// terminal error. `Cancelled` causes the supervisor to exit cleanly
/// without sending the current notification.
#[derive(Debug)]
pub(super) enum DispatchOutcome {
    /// A required trigger failed after all retries.
    RequiredFailed {
        kind: TriggerKindLabel,
        source: TriggerError,
    },
    /// Parent or per-stream cancellation fired between triggers or during
    /// a retry backoff sleep.
    Cancelled,
}
