//! Trigger dispatch and retry loop.

use std::time::Duration;

use tokio::sync::{oneshot, watch};

use crate::Notification;

use super::command::dispatch_command;
use super::echo::dispatch_echo;
use super::kind::{TriggerKind, trigger_kind_label};
use super::log::dispatch_log;
use super::{DispatchOutcome, Trigger, TriggerError, TriggerState};
use crate::watch::backoff::compute_backoff;
use crate::watch::outcome::ReconnectPolicy;

#[cfg(test)]
mod tests;

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
async fn dispatch_triggers_with_backoff<F>(
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
        if check_cancelled(parent_cancel, cancel) {
            return Err(DispatchOutcome::Cancelled);
        }

        let mut attempt: u32 = 0;
        let outcome = loop {
            match dispatch_one_attempt(trigger, state, notification).await {
                Ok(()) => break Ok(()),
                Err(err) => {
                    if is_terminal_error(trigger, &err) || attempt >= trigger.retries {
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

async fn dispatch_one_attempt(
    trigger: &Trigger,
    state: &mut TriggerState,
    notification: &Notification,
) -> Result<(), TriggerError> {
    match &trigger.kind {
        TriggerKind::Echo => dispatch_echo(notification),
        TriggerKind::Log { path } => dispatch_log(path, state, notification).await,
        TriggerKind::Command(cfg) => dispatch_command(cfg, trigger.timeout, notification).await,
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

/// Decide whether an attempt error should terminate the retry loop
/// immediately (bypassing the retry budget) or stay retryable.
///
/// `fail_fast = false` keeps every failure retryable. `fail_fast =
/// true` (the default) treats every `TriggerError::Command` (non-zero
/// exit) and every `TriggerError::Template` (any render-time failure:
/// missing notification path, missing env var, env var not unicode,
/// malformed template, or notification encode failure) as terminal
/// because they are deterministic with respect to the current
/// notification and process environment: the same input produces the
/// same failure, so retrying wastes the budget. `Io`, `Encode`, and
/// `Timeout` stay retryable because they are genuinely transient
/// (broken pipe, disk transiently full, slow downstream).
fn is_terminal_error(trigger: &Trigger, err: &TriggerError) -> bool {
    if !trigger.fail_fast {
        return false;
    }
    matches!(
        err,
        TriggerError::Command { .. } | TriggerError::Template { .. }
    )
}

#[cfg(test)]
fn dispatch_test_failing(
    failures_remaining: &std::sync::Arc<std::sync::atomic::AtomicU32>,
    eventual: &super::kind::TestEventual,
) -> Result<(), TriggerError> {
    use std::sync::atomic::Ordering;
    let prev = failures_remaining.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
        if v > 0 { Some(v - 1) } else { None }
    });
    if prev.is_ok() {
        return Err(TriggerError::Io(std::io::Error::other("test failure")));
    }
    match eventual {
        super::kind::TestEventual::Succeed => Ok(()),
        super::kind::TestEventual::Fail => Err(TriggerError::Io(std::io::Error::other(
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
