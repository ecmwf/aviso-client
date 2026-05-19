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
pub use template::TemplateErrorKind;

/// Command trigger dispatch.
mod command;
/// Trigger dispatch orchestration.
mod dispatcher;
/// Echo trigger dispatch.
mod echo;
/// Trigger kind internals.
mod kind;
/// Log trigger dispatch.
mod log;
/// Template substitution engine shared by command and webhook triggers.
mod template;

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
            timeout: None,
            fail_fast: true,
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
            timeout: None,
            fail_fast: true,
        }
    }

    /// Build a command trigger that runs `/bin/sh -c <cmd>` once per
    /// notification, with the notification's fields exposed as
    /// `AVISO_*` environment variables. The command string is
    /// rendered through the trigger template engine: `{{ notification.<path> }}`
    /// substitutes a notification field and `{{ env.<NAME> }}` reads
    /// from the process environment.
    ///
    /// # Environment variable injection
    ///
    /// The dispatcher injects `AVISO_EVENT_TYPE`, `AVISO_SEQUENCE`,
    /// `AVISO_REQUEST_ID` (when present), `AVISO_IDENTIFIER_<KEY>`
    /// per identifier entry (uppercased, non-alphanumerics replaced
    /// with `_`), `AVISO_PAYLOAD_JSON` (full payload as compact JSON),
    /// and `AVISO_NOTIFICATION_JSON` (full notification as compact
    /// JSON). User-supplied env vars via [`Self::env`] are applied
    /// AFTER the dispatcher-injected vars, so user keys override
    /// dispatcher keys when both are present.
    ///
    /// # Output capture
    ///
    /// Stdout and stderr are captured concurrently into ring buffers
    /// of 4 KiB. Command stdout content is dropped per the
    /// no-payload-logging discipline; only the captured byte count
    /// reaches DEBUG-level tracing. Stderr tail goes into the public
    /// [`crate::ClientError::TriggerFailed`] variant on non-zero
    /// exit.
    ///
    /// # Shell descendant cleanup
    ///
    /// The dispatcher kills the `/bin/sh -c ...` child on timeout
    /// (when a per-trigger timeout setter wires in on this branch)
    /// but does NOT propagate the kill signal to pipelines,
    /// backgrounded jobs, or grandchildren. Operators who need full
    /// process-tree cleanup should use `exec ./binary` so the shell
    /// PID equals the target binary's PID.
    ///
    /// # POSIX-only
    ///
    /// Supports unix only; Windows support is deferred.
    ///
    /// # Template errors
    ///
    /// The constructor is infallible. A malformed template (unclosed
    /// braces, unknown namespace, etc.) surfaces at first dispatch as
    /// [`TriggerError::Template`].
    ///
    /// Defaults: `retries: 0`, `required: true`.
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
    /// process. Repeatable; later sets override earlier ones with the
    /// same key.
    ///
    /// Has no effect on echo or log triggers; only the command
    /// trigger honours it.
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
    ///
    /// Has no effect on echo or log triggers; only the command
    /// trigger honours it.
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
    /// Meaningful for the command trigger only: bounds the wait on the
    /// child process with `tokio::time::sleep` raced against
    /// `child.wait()`; on expiry the dispatcher issues `SIGKILL`,
    /// reaps the zombie, and returns [`TriggerError::Timeout`].
    ///
    /// Has no effect on echo or log triggers: their dispatchers
    /// complete in microseconds (locked-stdout write or
    /// `tokio::fs::File::write_all` flush), and a non-preemptible
    /// sync syscall cannot be interrupted by a separate sleep
    /// future. The field is silently ignored on those kinds.
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
    /// Meaningful for the command trigger only:
    /// [`TriggerError::Command`] (non-zero exit) and
    /// [`TriggerError::Template`] (malformed template) are terminal
    /// under `fail_fast = true` because they are deterministic
    /// (the same input produces the same failure); `Io` and
    /// `Timeout` stay retryable because they are genuinely
    /// transient.
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

/// Human-readable label naming the kind of trigger that produced a
/// [`crate::ClientError::TriggerFailed`].
///
/// Separate from the crate-private `TriggerKind` enum so future internal
/// variants (or test-only ones) cannot leak into public error displays.
///
/// The `Command` variant intentionally carries no body: a command
/// trigger's full command string may contain secrets (bearer tokens,
/// connection URIs), and any redacted summary is still an attack
/// surface if the secret appears in the visible prefix. The label
/// displays as the bare string `"command"`; the full command goes
/// into DEBUG-level structured tracing only.
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
    /// The command trigger (subprocess spawn). Carries no body to
    /// avoid leaking secret-bearing command fragments through error
    /// chains; the full command appears in DEBUG-level tracing only.
    Command,
}

impl std::fmt::Display for TriggerKindLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo => f.write_str("echo"),
            Self::Log { path } => write!(f, "log({})", path.display()),
            Self::Command => f.write_str("command"),
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

    /// A command trigger's child process exited with a non-zero
    /// status. `stderr_tail` is the last 4 KiB of the child's stderr,
    /// captured into a ring buffer; the head is dropped on overflow.
    /// Stdout content is suppressed per the no-payload-logging rule.
    #[error("command exited {exit_code}: {stderr_tail}")]
    Command {
        /// Child process's exit code. `-1` when the child died from
        /// a signal (Unix sets the exit code to None for
        /// signal-terminated children; `-1` is the canonical sentinel).
        exit_code: i32,
        /// Last 4 KiB of the child's stderr, lossily UTF-8 decoded.
        stderr_tail: String,
    },

    /// A trigger attempt exceeded its configured per-trigger timeout.
    ///
    /// Surfaced only on triggers that have a meaningful timeout
    /// (currently the command trigger; echo and log silently ignore
    /// the [`Trigger::timeout`] setter). The carried duration is the
    /// timeout that was set, not the actual elapsed time.
    #[error("trigger timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// A template substitution failed.
    ///
    /// The `context` is a safe static label naming WHICH template
    /// surface produced the error (`"command"`, `"webhook url"`,
    /// `"webhook body"`, `"webhook header"`). It is NOT a snippet of
    /// the raw template source: raw templates may carry secrets
    /// (e.g., a bearer token baked into a webhook URL), so only safe
    /// labels reach the public error chain.
    ///
    /// The `field` names the specific path or env-var name that
    /// failed: a JSON path like `"notification.payload.target"` for
    /// `Missing`, an env-var name like `"SLACK_TOKEN"` for `EnvNotSet`,
    /// or a safe static label like `"unclosed_braces"` for `BadSyntax`.
    /// For `BadSyntax` specifically, the `field` carries no fragment
    /// from the raw template; it names only the parse failure
    /// category.
    ///
    /// The raw template source is logged at `DEBUG` via the
    /// `client.trigger.template.render_failed` tracing event for
    /// operators who control the logging sink, but never appears in
    /// this public variant.
    #[error("template render in {context} failed at {field}: {kind:?}")]
    Template {
        /// Safe static label naming which template surface failed.
        context: String,
        /// Specific path or env-var name that failed; for
        /// `BadSyntax`, a safe static label naming the parse failure.
        field: String,
        /// Categorisation of the failure.
        kind: TemplateErrorKind,
    },
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
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn trigger_error_encode_variant_carries_serde_error() {
        let parse_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let err: TriggerError = parse_err.into();
        assert!(matches!(err, TriggerError::Encode(_)));
    }

    #[test]
    fn trigger_error_template_display_uses_safe_context_not_raw_template() {
        use crate::watch::TemplateErrorKind;
        let err = TriggerError::Template {
            context: "command".to_string(),
            field: "notification.payload.target".to_string(),
            kind: TemplateErrorKind::Missing,
        };
        let rendered = err.to_string();
        assert!(rendered.contains("command"), "got: {rendered}");
        assert!(
            rendered.contains("notification.payload.target"),
            "got: {rendered}"
        );
        assert!(rendered.contains("Missing"), "got: {rendered}");
    }
}
