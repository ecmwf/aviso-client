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
/// true` (the default) treats `TriggerError::Command` (non-zero exit)
/// and `TriggerError::Template` (malformed template) as terminal
/// because they are deterministic; the same input produces the same
/// failure, so retrying wastes the budget. `Io`, `Encode`, and
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on channel send and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use tokio::sync::{oneshot, watch};

    use super::dispatch_triggers_with_backoff;
    use crate::Notification;
    use crate::watch::TriggerError;
    use crate::watch::trigger::command::build_command_config;
    use crate::watch::trigger::kind::{TestEventual, TriggerKind};
    use crate::watch::trigger::{DispatchOutcome, Trigger, TriggerState};

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

        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        drop_tx.send(true).unwrap();
        let result = fut.await;
        assert!(matches!(result, Err(DispatchOutcome::Cancelled)));
    }

    #[tokio::test]
    async fn command_terminal_failure_short_circuits_retries_with_fail_fast_true() {
        let cfg = build_command_config("exit 7");
        let trigger = Trigger {
            kind: TriggerKind::Command(Box::new(cfg)),
            retries: 5,
            required: true,
            timeout: None,
            fail_fast: true,
        };
        let mut states = vec![TriggerState::new()];
        let started = std::time::Instant::now();
        let result = run_once(&[trigger], &mut states, |_| Duration::from_secs(60)).await;
        let elapsed = started.elapsed();
        assert!(
            matches!(
                result,
                Err(DispatchOutcome::RequiredFailed {
                    source: TriggerError::Command { exit_code: 7, .. },
                    ..
                })
            ),
            "got: {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "fail_fast=true must short-circuit retries; took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn command_template_error_is_terminal_with_fail_fast_true() {
        let cfg = build_command_config("hello {{ notification.event_type");
        let trigger = Trigger {
            kind: TriggerKind::Command(Box::new(cfg)),
            retries: 5,
            required: true,
            timeout: None,
            fail_fast: true,
        };
        let mut states = vec![TriggerState::new()];
        let started = std::time::Instant::now();
        let result = run_once(&[trigger], &mut states, |_| Duration::from_secs(60)).await;
        let elapsed = started.elapsed();
        assert!(
            matches!(
                result,
                Err(DispatchOutcome::RequiredFailed {
                    source: TriggerError::Template { .. },
                    ..
                })
            ),
            "got: {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "template errors are deterministic; must short-circuit"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn command_nonzero_exit_retries_when_fail_fast_false() {
        let cfg = build_command_config("exit 1");
        let trigger = Trigger {
            kind: TriggerKind::Command(Box::new(cfg)),
            retries: 2,
            required: true,
            timeout: None,
            fail_fast: false,
        };
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
        for _ in 0..2 {
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_millis(110)).await;
        }
        let result = fut.await;
        match result {
            Err(DispatchOutcome::RequiredFailed {
                source: TriggerError::Command { exit_code: 1, .. },
                ..
            }) => {}
            other => panic!("expected RequiredFailed after retries exhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn command_timeout_returns_timeout_error_after_kill_and_reap() {
        let cfg = build_command_config("sleep 30");
        let trigger = Trigger {
            kind: TriggerKind::Command(Box::new(cfg)),
            retries: 0,
            required: true,
            timeout: Some(Duration::from_millis(200)),
            fail_fast: true,
        };
        let mut states = vec![TriggerState::new()];
        let started = std::time::Instant::now();
        let result = run_once(&[trigger], &mut states, |_| Duration::from_millis(1)).await;
        let elapsed = started.elapsed();
        assert!(
            matches!(
                result,
                Err(DispatchOutcome::RequiredFailed {
                    source: TriggerError::Timeout(_),
                    ..
                })
            ),
            "got: {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout must kill the child quickly; took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn parent_cancel_between_triggers_returns_cancelled() {
        let echo1 = Trigger::echo();
        let echo2 = Trigger::echo();
        let mut states = vec![TriggerState::new(), TriggerState::new()];
        let (drop_tx, mut parent_rx) = watch::channel(false);
        let (_cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
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
