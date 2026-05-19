//! Trigger configuration types and dispatcher for watch sessions.
//!
//! A [`Trigger`] is a per-notification side effect attached to a
//! [`crate::watch::WatchRequest`] via
//! [`crate::watch::WatchRequest::with_triggers`]. Two built-in kinds ship in
//! the first release:
//!
//! - [`Trigger::echo`]: writes the notification as NDJSON to standard output.
//! - [`Trigger::log`]: appends the notification as NDJSON to a user-specified
//!   file.
//!
//! Each trigger has a `retries` count (default `0`) and a `required` flag
//! (default `true`). A required trigger that fails after all retries
//! terminates the watch with [`crate::ClientError::TriggerFailed`]; an optional
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

/// Trigger dispatch orchestration.
mod dispatcher;
/// Echo trigger dispatch.
mod echo;
/// Trigger kind internals.
mod kind;
/// Log trigger dispatch.
mod log;

/// A single trigger configured on a watch.
///
/// Built via [`Self::echo`] or [`Self::log`]; tuned via the chainable
/// [`Self::retries`] and [`Self::required`] setters. Defaults are the
/// safe choice for an at-least-once system: required (failure terminates
/// the watch), zero retries.
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
        } = self;
        f.debug_struct("Trigger")
            .field("kind", kind)
            .field("retries", retries)
            .field("required", required)
            .finish()
    }
}

impl Trigger {
    /// Build an echo trigger that writes each notification as a single line
    /// of compact JSON to standard output.
    ///
    /// Defaults: `retries: 0`, `required: true`.
    #[must_use]
    pub fn echo() -> Self {
        Self {
            kind: TriggerKind::Echo,
            retries: 0,
            required: true,
        }
    }

    /// Build a log trigger that appends each notification as a single line of
    /// compact JSON to the file at `path`. The file is opened with
    /// `append(true).create(true)` on first dispatch and held open for the
    /// trigger's lifetime; no rotation, no per-write `fsync`.
    ///
    /// Defaults: `retries: 0`, `required: true`.
    #[must_use]
    pub fn log(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: TriggerKind::Log { path: path.into() },
            retries: 0,
            required: true,
        }
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

/// Human-readable label naming the kind of trigger that produced a
/// [`crate::ClientError::TriggerFailed`].
///
/// Separate from the crate-private `TriggerKind` enum so future internal
/// variants (or test-only ones) cannot leak into public error displays.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerKindLabel {
    /// The echo trigger (NDJSON to standard output).
    Echo,
    /// The log trigger (NDJSON appended to a file at the carried path).
    Log {
        /// The configured log file path.
        path: PathBuf,
    },
}

impl std::fmt::Display for TriggerKindLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo => f.write_str("echo"),
            Self::Log { path } => write!(f, "log({})", path.display()),
        }
    }
}

/// Error returned by a single trigger dispatch attempt.
///
/// Carried as the `source` of [`crate::ClientError::TriggerFailed`] when a required
/// trigger fails. The variants cover the failure modes the v1 dispatcher
/// can produce; the enum is `#[non_exhaustive]` so future triggers (for
/// example a Webhook trigger with HTTP-status awareness) can add variants
/// without breaking downstream matches.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TriggerError {
    /// Underlying I/O failure (broken pipe writing to stdout, permission
    /// denied opening the log file, disk full, file vanished, and so on).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// `serde_json` refused to serialise the notification. Highly unlikely
    /// for the well-typed `Notification` shape, but the variant exists for
    /// safety.
    #[error("encode notification: {0}")]
    Encode(#[from] serde_json::Error),
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on constructor success and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use std::path::PathBuf;

    use super::{Trigger, TriggerError, TriggerKindLabel};
    use crate::watch::trigger::kind::TriggerKind;

    #[test]
    fn echo_constructor_uses_default_retries_zero_and_required_true() {
        let trigger = Trigger::echo();
        assert!(matches!(trigger.kind, TriggerKind::Echo));
        assert_eq!(trigger.retries, 0);
        assert!(trigger.required);
    }

    #[test]
    fn log_constructor_uses_default_retries_zero_and_required_true() {
        let trigger = Trigger::log("/tmp/some.log");
        let TriggerKind::Log { path } = &trigger.kind else {
            panic!("expected Log variant");
        };
        assert_eq!(path, &PathBuf::from("/tmp/some.log"));
        assert_eq!(trigger.retries, 0);
        assert!(trigger.required);
    }

    #[test]
    fn retries_setter_overrides_default() {
        let trigger = Trigger::echo().retries(7);
        assert_eq!(trigger.retries, 7);
        assert!(matches!(trigger.kind, TriggerKind::Echo));
        assert!(trigger.required);
    }

    #[test]
    fn required_setter_overrides_default() {
        let trigger = Trigger::echo().required(false);
        assert!(!trigger.required);
        assert_eq!(trigger.retries, 0);
    }

    #[test]
    fn trigger_clone_preserves_all_fields() {
        let original = Trigger::log("/tmp/clone.log").retries(3).required(false);
        let cloned = original.clone();
        let (TriggerKind::Log { path: a }, TriggerKind::Log { path: b }) =
            (&original.kind, &cloned.kind)
        else {
            panic!("clone did not preserve Log variant");
        };
        assert_eq!(a, b);
        assert_eq!(cloned.retries, original.retries);
        assert_eq!(cloned.required, original.required);
    }

    #[test]
    fn trigger_debug_includes_all_fields() {
        let trigger = Trigger::log("/tmp/dbg.log").retries(2).required(true);
        let dbg = format!("{trigger:?}");
        assert!(dbg.contains("Trigger"), "got: {dbg}");
        assert!(dbg.contains("kind"), "got: {dbg}");
        assert!(dbg.contains("retries"), "got: {dbg}");
        assert!(dbg.contains("required"), "got: {dbg}");
        assert!(dbg.contains("/tmp/dbg.log"), "got: {dbg}");
    }

    #[test]
    fn trigger_kind_label_display_for_echo() {
        let label = TriggerKindLabel::Echo;
        assert_eq!(label.to_string(), "echo");
    }

    #[test]
    fn trigger_kind_label_display_for_log_includes_path() {
        let label = TriggerKindLabel::Log {
            path: PathBuf::from("/var/log/aviso.log"),
        };
        assert_eq!(label.to_string(), "log(/var/log/aviso.log)");
    }

    #[test]
    fn trigger_error_io_variant_carries_io_kind() {
        let err: TriggerError = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe").into();
        match err {
            TriggerError::Io(inner) => assert_eq!(inner.kind(), std::io::ErrorKind::BrokenPipe),
            TriggerError::Encode(e) => panic!("expected Io, got Encode: {e}"),
        }
    }

    #[test]
    fn trigger_error_encode_variant_carries_serde_error() {
        let parse_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let err: TriggerError = parse_err.into();
        assert!(matches!(err, TriggerError::Encode(_)));
    }
}
