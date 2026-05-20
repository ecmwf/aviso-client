//! Unit tests for the trigger dispatcher: retry loop, backoff
//! injection, terminal-vs-retryable classifier (`fail_fast`),
//! parent-drop cancellation, and the `cfg(test)` `Test*` variant
//! dispatch paths.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on channel send and panic on unexpected variant are the standard test diagnostics"
)]

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::{oneshot, watch};

use super::dispatch_triggers_with_backoff;
use crate::Notification;
use crate::watch::TriggerError;
#[cfg(unix)]
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
    let http = test_http_client();
    dispatch_triggers_with_backoff(
        triggers,
        states,
        &n,
        &mut parent_rx,
        &mut cancel_rx,
        &http,
        backoff,
    )
    .await
}

/// Build a fresh reqwest client for dispatcher tests. The test
/// triggers (`TestFailing`, `TestFailOnCall`) and the production
/// echo/log/command dispatchers do not use the client; webhook
/// tests live in `webhook/tests.rs` against a real wiremock
/// server. The dispatcher loop just threads `&Client` through.
fn test_http_client() -> reqwest::Client {
    reqwest::Client::new()
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
    let http = test_http_client();
    let fut = dispatch_triggers_with_backoff(
        std::slice::from_ref(&trigger),
        &mut states,
        &n,
        &mut parent_rx,
        &mut cancel_rx,
        &http,
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
    let http = test_http_client();
    let fut = dispatch_triggers_with_backoff(
        std::slice::from_ref(&trigger),
        &mut states,
        &n,
        &mut parent_rx,
        &mut cancel_rx,
        &http,
        |_| Duration::from_secs(60),
    );
    tokio::pin!(fut);

    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    drop_tx.send(true).unwrap();
    let result = fut.await;
    assert!(matches!(result, Err(DispatchOutcome::Cancelled)));
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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
    let http = test_http_client();
    let fut = dispatch_triggers_with_backoff(
        std::slice::from_ref(&trigger),
        &mut states,
        &n,
        &mut parent_rx,
        &mut cancel_rx,
        &http,
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

#[cfg(unix)]
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
    let http = test_http_client();
    let result = dispatch_triggers_with_backoff(
        &[echo1, echo2],
        &mut states,
        &n,
        &mut parent_rx,
        &mut cancel_rx,
        &http,
        |_| Duration::from_millis(1),
    )
    .await;
    assert!(matches!(result, Err(DispatchOutcome::Cancelled)));
}

#[tokio::test]
async fn webhook_4xx_is_terminal_with_fail_fast_true() {
    use crate::watch::trigger::webhook::build_webhook_config;
    let cfg = build_webhook_config("http://127.0.0.1:1/unused");
    let trigger = Trigger {
        kind: TriggerKind::Webhook(Box::new(cfg)),
        retries: 5,
        required: true,
        timeout: None,
        fail_fast: true,
    };
    let err = TriggerError::Webhook {
        status: Some(reqwest::StatusCode::BAD_REQUEST),
        body_tail: String::new(),
    };
    assert!(
        super::is_terminal_error(&trigger, &err),
        "4xx must be terminal under fail_fast = true"
    );
}

#[tokio::test]
async fn webhook_5xx_is_retryable_with_fail_fast_true() {
    use crate::watch::trigger::webhook::build_webhook_config;
    let cfg = build_webhook_config("http://127.0.0.1:1/unused");
    let trigger = Trigger {
        kind: TriggerKind::Webhook(Box::new(cfg)),
        retries: 5,
        required: true,
        timeout: None,
        fail_fast: true,
    };
    let err = TriggerError::Webhook {
        status: Some(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
        body_tail: String::new(),
    };
    assert!(
        !super::is_terminal_error(&trigger, &err),
        "5xx must stay retryable even under fail_fast = true"
    );
}

#[tokio::test]
async fn webhook_transport_error_is_retryable_with_fail_fast_true() {
    use crate::watch::trigger::webhook::build_webhook_config;
    let cfg = build_webhook_config("http://127.0.0.1:1/unused");
    let trigger = Trigger {
        kind: TriggerKind::Webhook(Box::new(cfg)),
        retries: 5,
        required: true,
        timeout: None,
        fail_fast: true,
    };
    let err = TriggerError::Webhook {
        status: None,
        body_tail: String::new(),
    };
    assert!(
        !super::is_terminal_error(&trigger, &err),
        "transport error (None status) must stay retryable"
    );
}

#[tokio::test]
async fn webhook_4xx_is_retryable_with_fail_fast_false() {
    use crate::watch::trigger::webhook::build_webhook_config;
    let cfg = build_webhook_config("http://127.0.0.1:1/unused");
    let trigger = Trigger {
        kind: TriggerKind::Webhook(Box::new(cfg)),
        retries: 5,
        required: true,
        timeout: None,
        fail_fast: false,
    };
    let err = TriggerError::Webhook {
        status: Some(reqwest::StatusCode::BAD_REQUEST),
        body_tail: String::new(),
    };
    assert!(
        !super::is_terminal_error(&trigger, &err),
        "fail_fast=false must keep every error retryable, including 4xx"
    );
}
