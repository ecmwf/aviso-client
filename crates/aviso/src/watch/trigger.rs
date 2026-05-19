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
//! configured trigger in declaration order, sequential, with the per-trigger
//! `retries` budget and the supervisor's `compute_backoff` schedule
//! between attempts. A single dispatch attempt runs to completion (it is the
//! atomic unit; the dispatcher does NOT race the attempt against
//! cancellation). Between attempts and between triggers, the dispatcher
//! honours both `parent_cancel` and the per-stream `cancel` oneshot.
//!
//! [`dispatch_triggers_with_backoff`] is the test-injectable inner form.
//! Production wraps it with `compute_backoff`; unit tests pass a
//! deterministic sleep function so `tokio::time::pause` + `advance` can
//! step over the backoff without depending on jitter.
//!
//! The error surface returned by trigger dispatch is [`TriggerError`]; the
//! human-readable kind tag that appears on
//! `ClientError::TriggerFailed` is [`TriggerKindLabel`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::{oneshot, watch};

use crate::Notification;

use super::backoff::compute_backoff;
use super::outcome::ReconnectPolicy;

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
    pub(crate) kind: TriggerKind,
    pub(crate) retries: u32,
    pub(crate) required: bool,
}

/// Manual `Debug` impl rather than `#[derive(Debug)]`: the derived form
/// does not count as a "production read" for rustc's `dead_code` lint,
/// so a private field that is only constructed but never read elsewhere
/// would still warn even with the derive. The destructure-then-name-each-
/// field pattern below is what the lint counts as a read. The dispatcher
/// in this file also reads `kind`, `retries`, and `required`, so the
/// manual impl is not strictly required today; it is kept so that the
/// "fields are intentionally used" invariant is documented at the type
/// declaration itself, surviving any future refactor that moves the
/// dispatcher out of this module.
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

/// Internal description of which built-in trigger a [`Trigger`] runs.
///
/// Crate-private; downstream callers configure a `Trigger` through the
/// public [`Trigger::echo`] and [`Trigger::log`] constructors, never by
/// naming this enum.
#[derive(Clone)]
pub(crate) enum TriggerKind {
    Echo,
    Log {
        path: PathBuf,
    },
    /// Test-only: fails the first `failures_remaining` attempts, then
    /// resolves per `eventual`. Used by unit tests to drive "fail K times
    /// then succeed/fail" patterns deterministically.
    #[cfg(test)]
    TestFailing {
        failures_remaining: std::sync::Arc<std::sync::atomic::AtomicU32>,
        eventual: TestEventual,
    },
    /// Test-only: fails on the Nth invocation across notifications and
    /// succeeds on all others. Used by unit tests to drive "succeed on
    /// N=1, fail on N=2" patterns that share a single trigger config.
    #[cfg(test)]
    TestFailOnCall {
        calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
        fail_on_call: u32,
    },
}

/// Resolution of a test-only [`TriggerKind::TestFailing`] after its
/// `failures_remaining` counter hits zero.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) enum TestEventual {
    /// Subsequent calls succeed.
    Succeed,
    /// Subsequent calls also fail (the trigger never recovers).
    Fail,
}

/// Manual `Debug` impl for the same reason as [`Trigger`]'s manual impl.
impl std::fmt::Debug for TriggerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo => f.debug_struct("Echo").finish(),
            Self::Log { path } => f.debug_struct("Log").field("path", path).finish(),
            #[cfg(test)]
            Self::TestFailing {
                failures_remaining,
                eventual,
            } => f
                .debug_struct("TestFailing")
                .field("failures_remaining", failures_remaining)
                .field("eventual", eventual)
                .finish(),
            #[cfg(test)]
            Self::TestFailOnCall {
                calls,
                fail_on_call,
            } => f
                .debug_struct("TestFailOnCall")
                .field("calls", calls)
                .field("fail_on_call", fail_on_call)
                .finish(),
        }
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
    pub(crate) fn test_failing(
        failures_remaining: u32,
        eventual: TestEventual,
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
pub(crate) struct TriggerState {
    pub(crate) log_handle: Option<tokio::fs::File>,
}

impl TriggerState {
    pub(crate) fn new() -> Self {
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
pub(crate) enum DispatchOutcome {
    /// A required trigger failed after all retries.
    RequiredFailed {
        kind: TriggerKindLabel,
        source: TriggerError,
    },
    /// Parent or per-stream cancellation fired between triggers or during
    /// a retry backoff sleep.
    Cancelled,
}

/// Run all configured triggers for a notification using the production
/// backoff schedule.
///
/// See module docs for the contract; this is a thin wrapper around
/// [`dispatch_triggers_with_backoff`] that wires the supervisor's
/// `compute_backoff` schedule.
pub(crate) async fn dispatch_triggers(
    triggers: &[Trigger],
    states: &mut [TriggerState],
    notification: &Notification,
    parent_cancel: &mut watch::Receiver<bool>,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<(), DispatchOutcome> {
    dispatch_triggers_with_backoff(
        triggers,
        states,
        notification,
        parent_cancel,
        cancel,
        |attempt| compute_backoff(attempt, ReconnectPolicy::ExponentialBackoff),
    )
    .await
}

/// Run all configured triggers using an injectable backoff function.
///
/// Unit tests pass a deterministic backoff (typically
/// `|_| Duration::from_millis(100)`) so `tokio::time::pause` plus
/// `tokio::time::advance` can step over retry sleeps without depending on
/// the production jitter that can legitimately return zero nanoseconds.
///
/// The `backoff` function is invoked with the zero-based retry attempt
/// index that just failed (so attempt 0 is the FIRST retry sleep after
/// the initial-attempt failure).
pub(crate) async fn dispatch_triggers_with_backoff<F>(
    triggers: &[Trigger],
    states: &mut [TriggerState],
    notification: &Notification,
    parent_cancel: &mut watch::Receiver<bool>,
    cancel: &mut oneshot::Receiver<()>,
    backoff: F,
) -> Result<(), DispatchOutcome>
where
    F: Fn(u32) -> Duration,
{
    debug_assert_eq!(
        triggers.len(),
        states.len(),
        "triggers and states must be aligned"
    );

    for (trigger, state) in triggers.iter().zip(states.iter_mut()) {
        // Cancel check between triggers.
        if check_cancelled(parent_cancel, cancel) {
            return Err(DispatchOutcome::Cancelled);
        }

        let mut attempt: u32 = 0;
        let outcome = loop {
            match dispatch_one_attempt(&trigger.kind, state, notification).await {
                Ok(()) => break Ok(()),
                Err(err) => {
                    if attempt >= trigger.retries {
                        break Err(err);
                    }
                    let delay = backoff(attempt);
                    let sleep = tokio::time::sleep(delay);
                    tokio::pin!(sleep);
                    tokio::select! {
                        biased;
                        _ = parent_cancel.changed() => return Err(DispatchOutcome::Cancelled),
                        _ = &mut *cancel => return Err(DispatchOutcome::Cancelled),
                        () = &mut sleep => {}
                    }
                    attempt = attempt.saturating_add(1);
                }
            }
        };

        if let Err(source) = outcome {
            let label = trigger_kind_label(&trigger.kind);
            if trigger.required {
                return Err(DispatchOutcome::RequiredFailed {
                    kind: label,
                    source,
                });
            }
            tracing::warn!(
                event.name = "client.trigger.failed",
                kind = %label,
                retries = trigger.retries,
                error = %source,
                "optional trigger failed; continuing"
            );
        }
    }
    Ok(())
}

/// Non-blocking cancel probe used between triggers.
///
/// Returns `true` when either cancellation source has fired:
///
/// - **Parent drop**: the watch `Sender` was dropped (all clones gone), in
///   which case `has_changed` returns `Err(_)`, OR the borrowed value is
///   already `true` (the `DropGuard` flipped it). Reading `*borrow()`
///   directly is robust to the case where an earlier `select!` arm
///   already consumed the change marker via `parent_cancel.changed()` and
///   left `has_changed()` returning `Ok(false)` while the value is still
///   `true`.
/// - **Per-stream cancel**: the oneshot has been signaled OR the sender
///   was dropped, observed via `try_recv` returning `Ok(())` or
///   `Err(Closed)`.
fn check_cancelled(
    parent_cancel: &mut watch::Receiver<bool>,
    cancel: &mut oneshot::Receiver<()>,
) -> bool {
    if parent_cancel.has_changed().is_err() || *parent_cancel.borrow() {
        return true;
    }
    matches!(
        cancel.try_recv(),
        Ok(()) | Err(oneshot::error::TryRecvError::Closed)
    )
}

/// Map an internal kind to its public-facing diagnostic label.
fn trigger_kind_label(kind: &TriggerKind) -> TriggerKindLabel {
    match kind {
        TriggerKind::Echo => TriggerKindLabel::Echo,
        TriggerKind::Log { path } => TriggerKindLabel::Log { path: path.clone() },
        #[cfg(test)]
        TriggerKind::TestFailing { .. } => TriggerKindLabel::Echo,
        #[cfg(test)]
        TriggerKind::TestFailOnCall { .. } => TriggerKindLabel::Echo,
    }
}

async fn dispatch_one_attempt(
    kind: &TriggerKind,
    state: &mut TriggerState,
    notification: &Notification,
) -> Result<(), TriggerError> {
    match kind {
        TriggerKind::Echo => dispatch_echo(notification),
        TriggerKind::Log { path } => dispatch_log(path, state, notification).await,
        #[cfg(test)]
        TriggerKind::TestFailing {
            failures_remaining,
            eventual,
        } => dispatch_test_failing(failures_remaining, eventual),
        #[cfg(test)]
        TriggerKind::TestFailOnCall {
            calls,
            fail_on_call,
        } => dispatch_test_fail_on_call(calls, *fail_on_call),
    }
}

/// Echo dispatch: serialise the notification into a buffer ONCE (appending
/// the newline to the same buffer), then a single `write_all` against a
/// locked stdout handle. Buffer-then-write avoids any intra-trigger seam
/// between the JSON body and the line terminator.
fn dispatch_echo(notification: &Notification) -> Result<(), TriggerError> {
    use std::io::Write as _;
    let mut buf = serde_json::to_vec(notification)?;
    buf.push(b'\n');
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(&buf)?;
    Ok(())
}

/// Log dispatch: lazy-open the file on first call, then buffer-then-write
/// the NDJSON line. No fsync per the at-least-once contract (a crashed
/// pre-commit log write replays on restart).
async fn dispatch_log(
    path: &Path,
    state: &mut TriggerState,
    notification: &Notification,
) -> Result<(), TriggerError> {
    use tokio::io::AsyncWriteExt as _;
    if state.log_handle.is_none() {
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .await?;
        state.log_handle = Some(file);
    }
    let Some(handle) = state.log_handle.as_mut() else {
        return Err(TriggerError::Io(std::io::Error::other(
            "log handle missing immediately after init; bug in lazy-open invariant",
        )));
    };
    let mut buf = serde_json::to_vec(notification)?;
    buf.push(b'\n');
    handle.write_all(&buf).await?;
    // tokio::fs::File buffers writes internally; flush so the kernel
    // sees the bytes immediately. Tailers and downstream NDJSON readers
    // benefit from prompt visibility, and the test suite relies on this
    // to read the file back synchronously. This is NOT an fsync; OS
    // pagecache durability is still asynchronous (the at-least-once
    // invariant covers crash replay, so no fsync per Q8).
    handle.flush().await?;
    Ok(())
}

#[cfg(test)]
fn dispatch_test_failing(
    failures_remaining: &std::sync::Arc<std::sync::atomic::AtomicU32>,
    eventual: &TestEventual,
) -> Result<(), TriggerError> {
    use std::sync::atomic::Ordering;
    let prev = failures_remaining.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
        if v > 0 { Some(v - 1) } else { None }
    });
    if prev.is_ok() {
        return Err(TriggerError::Io(std::io::Error::other("test failure")));
    }
    match eventual {
        TestEventual::Succeed => Ok(()),
        TestEventual::Fail => Err(TriggerError::Io(std::io::Error::other(
            "test eventual fail",
        ))),
    }
}

#[cfg(test)]
fn dispatch_test_fail_on_call(
    calls: &std::sync::Arc<std::sync::atomic::AtomicU32>,
    fail_on_call: u32,
) -> Result<(), TriggerError> {
    use std::sync::atomic::Ordering;
    let n = calls.fetch_add(1, Ordering::AcqRel) + 1;
    if n == fail_on_call {
        Err(TriggerError::Io(std::io::Error::other("test fail on call")))
    } else {
        Ok(())
    }
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

    use super::{Trigger, TriggerError, TriggerKind, TriggerKindLabel};

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
    fn trigger_kind_debug_includes_log_path() {
        let echo_dbg = format!("{:?}", TriggerKind::Echo);
        assert!(echo_dbg.contains("Echo"));

        let log_dbg = format!(
            "{:?}",
            TriggerKind::Log {
                path: PathBuf::from("/tmp/x.log")
            }
        );
        assert!(log_dbg.contains("Log"), "got: {log_dbg}");
        assert!(log_dbg.contains("/tmp/x.log"), "got: {log_dbg}");
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

    mod dispatch {
        use std::collections::BTreeMap;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        use tokio::sync::{oneshot, watch};

        use super::super::{
            DispatchOutcome, TestEventual, Trigger, TriggerKindLabel, TriggerState,
            dispatch_triggers_with_backoff,
        };
        use crate::Notification;

        fn make_notification() -> Notification {
            Notification {
                event_type: "mars".to_string(),
                sequence: 1,
                identifier: BTreeMap::new(),
                payload: serde_json::Value::Null,
                request_id: None,
            }
        }

        async fn run_once<F>(
            triggers: &[Trigger],
            states: &mut [TriggerState],
            backoff: F,
        ) -> Result<(), DispatchOutcome>
        where
            F: Fn(u32) -> Duration,
        {
            let (_drop_tx, mut parent_rx) = watch::channel(false);
            let (_cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
            let n = make_notification();
            dispatch_triggers_with_backoff(
                triggers,
                states,
                &n,
                &mut parent_rx,
                &mut cancel_rx,
                backoff,
            )
            .await
        }

        #[tokio::test]
        async fn echo_trigger_succeeds_without_retry() {
            let trigger = Trigger::echo();
            let mut states = vec![TriggerState::new()];
            let result = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
            assert!(matches!(result, Ok(())));
        }

        #[tokio::test]
        async fn log_trigger_writes_ndjson_to_tempfile() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("notif.log");
            let trigger = Trigger::log(&path);
            let mut states = vec![TriggerState::new()];
            let result = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
            assert!(matches!(result, Ok(())));
            // Drop the handle so the file's writes flush to disk before
            // we read it back (tokio::fs::File on Drop closes the fd).
            drop(states);
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(contents.starts_with('{'), "got: {contents}");
            assert!(
                contents.contains("\"event_type\":\"mars\""),
                "got: {contents}"
            );
            assert!(contents.ends_with('\n'), "got: {contents}");
        }

        #[tokio::test]
        async fn log_trigger_returns_io_error_when_parent_dir_missing() {
            let trigger = Trigger::log("/nonexistent-dir-aviso-test/x.log");
            let mut states = vec![TriggerState::new()];
            let result = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
            match result {
                Err(DispatchOutcome::RequiredFailed { kind, source }) => {
                    assert!(matches!(kind, TriggerKindLabel::Log { .. }));
                    let rendered = source.to_string();
                    assert!(rendered.starts_with("io:"), "got: {rendered}");
                }
                other => panic!("expected RequiredFailed, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn log_trigger_lazy_opens_then_reuses_handle_across_dispatches() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("reuse.log");
            let trigger = Trigger::log(&path);
            let mut states = vec![TriggerState::new()];
            assert!(states[0].log_handle.is_none());
            let _ = run_once(std::slice::from_ref(&trigger), &mut states, |_| {
                Duration::from_millis(1)
            })
            .await;
            assert!(states[0].log_handle.is_some(), "handle should be open");
            let handle_ptr_before: *const _ = states[0].log_handle.as_ref().unwrap();
            let _ = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
            let handle_ptr_after: *const _ = states[0].log_handle.as_ref().unwrap();
            assert!(std::ptr::eq(handle_ptr_before, handle_ptr_after));
        }

        #[tokio::test]
        async fn retries_exhausted_returns_required_failed_with_io_source() {
            let (trigger, counter) = Trigger::test_failing(5, TestEventual::Succeed, 2, true);
            let mut states = vec![TriggerState::new()];
            let result = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
            match result {
                Err(DispatchOutcome::RequiredFailed { source, .. }) => {
                    assert!(source.to_string().starts_with("io:"));
                }
                other => panic!("expected RequiredFailed, got {other:?}"),
            }
            // 3 attempts (initial + 2 retries) all fail, decrementing
            // failures_remaining from 5 down to 2.
            assert_eq!(counter.load(Ordering::Acquire), 2);
        }

        #[tokio::test]
        async fn retries_zero_fails_on_first_attempt() {
            let (trigger, counter) = Trigger::test_failing(1, TestEventual::Succeed, 0, true);
            let mut states = vec![TriggerState::new()];
            let result = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
            assert!(matches!(
                result,
                Err(DispatchOutcome::RequiredFailed { .. })
            ));
            assert_eq!(counter.load(Ordering::Acquire), 0);
        }

        #[tokio::test(start_paused = true)]
        async fn success_after_retry_advances_through_backoff_and_completes() {
            let (trigger, counter) = Trigger::test_failing(2, TestEventual::Succeed, 3, true);
            let mut states = vec![TriggerState::new()];
            let (_drop_tx, mut parent_rx) = watch::channel(false);
            let (_cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
            let n = make_notification();
            let fut = dispatch_triggers_with_backoff(
                std::slice::from_ref(&trigger),
                &mut states,
                &n,
                &mut parent_rx,
                &mut cancel_rx,
                |_| Duration::from_millis(100),
            );
            tokio::pin!(fut);

            // Step over both backoff sleeps (2 failures => 2 sleeps).
            for _ in 0..2 {
                tokio::task::yield_now().await;
                tokio::time::advance(Duration::from_millis(110)).await;
            }
            let result = fut.await;
            assert!(matches!(result, Ok(())), "got: {result:?}");
            assert_eq!(counter.load(Ordering::Acquire), 0);
        }

        #[tokio::test]
        async fn optional_trigger_failure_logs_warn_does_not_short_circuit() {
            let (failing_trigger, _) = Trigger::test_failing(5, TestEventual::Fail, 0, false);
            let success_trigger = Trigger::echo();
            let mut states = vec![TriggerState::new(), TriggerState::new()];
            let result = run_once(&[failing_trigger, success_trigger], &mut states, |_| {
                Duration::from_millis(1)
            })
            .await;
            assert!(matches!(result, Ok(())));
        }

        #[tokio::test(start_paused = true)]
        async fn parent_cancel_during_retry_backoff_returns_cancelled() {
            let (trigger, _counter) = Trigger::test_failing(1, TestEventual::Succeed, 3, true);
            let mut states = vec![TriggerState::new()];
            let (drop_tx, mut parent_rx) = watch::channel(false);
            let (_cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
            let n = make_notification();
            let fut = dispatch_triggers_with_backoff(
                std::slice::from_ref(&trigger),
                &mut states,
                &n,
                &mut parent_rx,
                &mut cancel_rx,
                |_| Duration::from_secs(60),
            );
            tokio::pin!(fut);

            // Let dispatch enter the retry backoff sleep (the first
            // attempt fails synchronously; the dispatcher then sleeps).
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            // Fire parent_cancel; the select arm must win over the
            // 60-second sleep.
            drop_tx.send(true).unwrap();
            let result = fut.await;
            assert!(matches!(result, Err(DispatchOutcome::Cancelled)));
        }

        #[tokio::test]
        async fn parent_cancel_between_triggers_returns_cancelled() {
            let echo1 = Trigger::echo();
            let echo2 = Trigger::echo();
            let mut states = vec![TriggerState::new(), TriggerState::new()];
            let (drop_tx, mut parent_rx) = watch::channel(false);
            let (_cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
            // Pre-fire the parent cancel before dispatch even starts; the
            // between-triggers cancel check fires immediately on the first
            // trigger (the dispatcher sees `has_changed()` true).
            drop_tx.send(true).unwrap();
            let n = make_notification();
            let result = dispatch_triggers_with_backoff(
                &[echo1, echo2],
                &mut states,
                &n,
                &mut parent_rx,
                &mut cancel_rx,
                |_| Duration::from_millis(1),
            )
            .await;
            assert!(matches!(result, Err(DispatchOutcome::Cancelled)));
        }
    }
}
