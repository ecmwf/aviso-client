// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};
use url::Url;

use super::connection::run_one_connection;
use super::{ActiveKeyGuard, ConnectionOutcome, PendingCommit, apply_outcome, send_or_cancel};
use crate::auth::AuthProvider;
use crate::state::{Checkpoint, ResumeKey, StateStore};
use crate::watch::backoff;
use crate::watch::{
    ConnectionLossReason, ConnectionStatus, FatalKind, ReconnectPolicy, ResumeStart, WatchEvent,
    WatchMode, WatchRequest, WatchState,
};
use crate::{ClientError, Notification};

/// A connection that ends sooner than this counts as a short session for
/// the purpose of slowing down Immediate reconnects. A routine
/// `max_duration_reached` close comes after minutes, not milliseconds.
const MIN_HEALTHY_SESSION: std::time::Duration = std::time::Duration::from_secs(1);

/// Drive a watch session through any number of reconnect cycles. Owns its
/// inputs by value; the spawn caller in
/// [`crate::client::AvisoClient::watch`] passes cloned or `Arc`-shared
/// handles so the task is `'static` without forcing the supervisor to
/// keep a [`crate::AvisoClient`] alive. Holding only the bits it needs
/// (rather than a cloned `AvisoClient`) is what lets parent-drop
/// cancellation work: the supervisor watches a `watch::Receiver<bool>`
/// subscribed from the client's `Arc<DropGuard>`, and when the last
/// client clone drops, the guard's `Drop` flips the channel and every
/// supervisor's `select!` arms observe the cancellation.
#[allow(
    clippy::too_many_lines,
    reason = "the outer reconnect loop is intentionally one function: each iteration's classification (close, http, transport, eof, heartbeat, fatal, cancel) and the wire-request rebuild belong together for readability; splitting them into helpers obscures the per-iteration state-mutation order"
)]
#[allow(
    clippy::too_many_arguments,
    reason = "the supervisor's collaborators are intentionally passed by value rather than bundled into a context struct, so each await point owns clear borrows; the supervisor is the only spawn caller's surface"
)]
pub(crate) async fn run_supervisor(
    request: WatchRequest,
    http: reqwest::Client,
    base_url: Url,
    auth: Option<Arc<dyn AuthProvider>>,
    heartbeat_interval: std::time::Duration,
    state_store: Option<Arc<dyn StateStore>>,
    resume_key: ResumeKey,
    tx: mpsc::Sender<Result<Notification, ClientError>>,
    mut cancel: oneshot::Receiver<()>,
    mut parent_cancel: watch::Receiver<bool>,
    active_resume_keys: Arc<std::sync::Mutex<std::collections::HashMap<ResumeKey, usize>>>,
    flush_cursor_on_exit: bool,
    done_signal: oneshot::Sender<()>,
    ready: watch::Sender<bool>,
) {
    struct DoneOnDrop(Option<oneshot::Sender<()>>);
    impl Drop for DoneOnDrop {
        fn drop(&mut self) {
            if let Some(s) = self.0.take() {
                let _ = s.send(());
            }
        }
    }
    let _done_on_drop = DoneOnDrop(Some(done_signal));
    let startup_timeout = request.startup_timeout();
    let readiness = ready.subscribe();
    let error_tx = tx.clone();
    let run = async {
        let _decrement_on_exit = ActiveKeyGuard {
            active: active_resume_keys.clone(),
            key: resume_key.clone(),
        };
        // Resolve initial cursor. User-supplied `request.from()` wins; if
        // absent and a state store is configured, query the store and resume
        // from its checkpoint. A store I/O failure surfaces as the first
        // stream item via `Err(ClientError::StateStore(_))`. The store read
        // is cancel-safe per `tokio::select!`.
        let initial_cursor: Option<ResumeStart> = match (request.from(), state_store.as_ref()) {
            (Some(_), _) => request.from().cloned(),
            (None, Some(store)) => {
                let get_result = tokio::select! {
                    biased;
                    _ = parent_cancel.changed() => return,
                    _ = &mut cancel => return,
                    r = store.get(&resume_key) => r,
                };
                match get_result {
                    Ok(Some(cp)) => {
                        // INFO level per D2 (plans/decisions.md): a successful
                        // resume from stored state is operator-visible
                        // information. The no-checkpoint-found path stays
                        // silent because starting fresh is the default.
                        tracing::info!(
                            event.name = "client.resume.applied",
                            resume_key = %resume_key.as_hex(),
                            sequence = cp.last_committed_sequence,
                            event_id = cp.last_event_id.as_deref(),
                            "resumed watch from stored checkpoint",
                        );
                        Some(ResumeStart::AfterSequence(cp.last_committed_sequence))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        let _ = send_or_cancel(
                            &tx,
                            Err(ClientError::from(e)),
                            &mut cancel,
                            &mut parent_cancel,
                        )
                        .await;
                        return;
                    }
                }
            }
            (None, None) => None,
        };

        // Build the reducer state from the resolved cursor (overriding the
        // request's `from()` view when the store provided one).
        let mut state = match (request.mode(), initial_cursor.clone()) {
            (WatchMode::Watch, from) => WatchState::watch(from),
            (WatchMode::ReplayOnly, Some(from)) => WatchState::replay_only(from),
            (WatchMode::ReplayOnly, None) => {
                let _ = send_or_cancel(
                    &tx,
                    Err(ClientError::Config(
                        "replay-only watch requires a resume position".into(),
                    )),
                    &mut cancel,
                    &mut parent_cancel,
                )
                .await;
                return;
            }
        };
        let mut last_reconnect_policy: Option<ReconnectPolicy> = None;
        let mut retry_counter: u32 = 0;
        // Consecutive connections that ended within MIN_HEALTHY_SESSION.
        // A server that accepts the watch and closes it at once with a
        // routine reason would otherwise be reconnected to with no delay,
        // as fast as the handshakes allow, for as long as it keeps doing
        // it. Short sessions turn an Immediate reconnect into a growing
        // backoff; one session of ordinary length resets the count.
        let mut short_sessions: u32 = 0;
        let mut retry_after_override: Option<std::time::Duration> = None;
        let mut commit_cursor: Option<u64> = initial_cursor.as_ref().and_then(|r| match r {
            ResumeStart::AfterSequence(n) => Some(*n),
            ResumeStart::Date(_) => None,
        });
        let mut pending_commit: Option<PendingCommit> = None;
        let mut refreshed_for_current_attempt: bool = false;
        let mut retry_cause = String::new();
        let mut last_retry_log: Option<tokio::time::Instant> = None;

        // Per-trigger mutable state held across notifications. Aligned with
        // `request.triggers()` by index. The log trigger's `tokio::fs::File`
        // handle lives here and is RAII-closed when this Vec is dropped on
        // supervisor exit.
        let mut trigger_states: Vec<crate::watch::trigger::TriggerState> = request
            .triggers()
            .iter()
            .map(|_| crate::watch::trigger::TriggerState::new())
            .collect();

        loop {
            if state.is_terminal() {
                break;
            }

            // Apply backoff sleep when the previous iteration captured a
            // reconnect policy. The first iteration's `last_reconnect_policy`
            // is `None`, so the initial connect proceeds without delay.
            if let Some(policy) = last_reconnect_policy.take() {
                // `retry_counter` counts failures-since-last-success; this is
                // the first iteration AFTER a failure, so subtracting one
                // gives `compute_backoff`'s 0-indexed retry-attempt semantic
                // (attempt = 0 for the first retry, [0, 250ms] window;
                // attempt = 1 for the second retry, [0, 500ms] window; etc.).
                // `saturating_sub` makes the routine `ServerClosed` reset
                // (`retry_counter = 0` + Immediate policy) also work since
                // `compute_backoff(0, Immediate)` returns ZERO regardless.
                let (attempt, policy) =
                    if policy == ReconnectPolicy::Immediate && short_sessions > 0 {
                        (
                            short_sessions.saturating_sub(1),
                            ReconnectPolicy::ExponentialBackoff,
                        )
                    } else {
                        (retry_counter.saturating_sub(1), policy)
                    };
                let delay = retry_after_override
                    .take()
                    .unwrap_or_else(|| backoff::compute_backoff(attempt, policy));
                if last_retry_log
                    .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(5))
                {
                    tracing::info!(
                        event.name = "client.connection.retrying",
                        event_type = request.event_type(),
                        cause = %retry_cause,
                        delay_ms = delay.as_millis(),
                        attempt = attempt.saturating_add(1),
                        "Retrying listener connection"
                    );
                    last_retry_log = Some(tokio::time::Instant::now());
                }
                if !delay.is_zero() {
                    let started = state.transition(WatchEvent::BackoffStarted(delay));
                    apply_outcome(&mut last_reconnect_policy, started);
                    let woke_for_cancel = tokio::select! {
                        biased;
                        _ = parent_cancel.changed() => true,
                        _ = &mut cancel => true,
                        () = tokio::time::sleep(delay) => false,
                    };
                    if woke_for_cancel {
                        break;
                    }
                    let elapsed = state.transition(WatchEvent::BackoffElapsed);
                    apply_outcome(&mut last_reconnect_policy, elapsed);
                }
            }

            // Auth refresh step: when the reducer is in `RefreshingAuth` the
            // previous iteration emitted `WatchEvent::AuthRejected` after a 401
            // and the supervisor now drives `AuthProvider::refresh()` before
            // attempting the next connection. D8's refresh-then-retry-once
            // contract is enforced by `refreshed_for_current_attempt`: the
            // flag is set after a successful refresh and reset on any
            // non-401 outcome, so a second 401 within the same attempt cycle
            // surfaces `AuthenticationRejectedAfterRefresh`.
            if matches!(state.connection_status(), ConnectionStatus::RefreshingAuth) {
                let Some(auth_provider) = auth.as_ref() else {
                    let outcome =
                        state.transition(WatchEvent::AuthRefreshCompleted { success: false });
                    apply_outcome(&mut last_reconnect_policy, outcome);
                    let _ = send_or_cancel(
                        &tx,
                        Err(ClientError::Auth(
                            "auth refresh requested but no auth provider is configured".into(),
                        )),
                        &mut cancel,
                        &mut parent_cancel,
                    )
                    .await;
                    break;
                };
                let refresh_result = tokio::select! {
                    biased;
                    _ = parent_cancel.changed() => break,
                    _ = &mut cancel => break,
                    r = auth_provider.refresh() => r,
                };
                match refresh_result {
                    Ok(()) => {
                        refreshed_for_current_attempt = true;
                        let outcome =
                            state.transition(WatchEvent::AuthRefreshCompleted { success: true });
                        apply_outcome(&mut last_reconnect_policy, outcome);
                    }
                    Err(e) => {
                        let outcome =
                            state.transition(WatchEvent::AuthRefreshCompleted { success: false });
                        apply_outcome(&mut last_reconnect_policy, outcome);
                        let _ = send_or_cancel(&tx, Err(e), &mut cancel, &mut parent_cancel).await;
                        break;
                    }
                }
            }

            // Build the wire-request cursor with this precedence:
            //   1. The persisted commit cursor (`commit_cursor`), once any
            //      notification has been committed.
            //   2. The highest successfully sent sequence in
            //      `pending_commit` (the supervisor sent it to the channel
            //      but the commit-on-next-send promotion has not run yet).
            //      Without this fallback, a `Date` initial cursor would be
            //      reused on reconnect even after one notification had been
            //      sent, contradicting D17's "from_date is bootstrap-only"
            //      contract and weakening cross-reconnect gap detection
            //      (the `GapGuard` would start without an expected next
            //      sequence and tolerate any starting value).
            //   3. The initial cursor (which may be a `Date` per D17
            //      bootstrap, valid only for the very first connection).
            let wire_from: Option<ResumeStart> = match (commit_cursor, pending_commit.as_ref()) {
                (Some(n), _) => Some(ResumeStart::AfterSequence(n)),
                (None, Some(p)) => Some(ResumeStart::AfterSequence(p.sequence)),
                (None, None) => initial_cursor.clone(),
            };

            let session_started = tokio::time::Instant::now();
            let outcome = run_one_connection(
                &mut state,
                &mut last_reconnect_policy,
                &request,
                wire_from.as_ref(),
                &mut commit_cursor,
                &mut pending_commit,
                state_store.as_ref(),
                &resume_key,
                &http,
                &base_url,
                auth.as_ref(),
                heartbeat_interval,
                &mut retry_counter,
                &mut trigger_states,
                &tx,
                &mut cancel,
                &mut parent_cancel,
                &ready,
            )
            .await;
            if session_started.elapsed() < MIN_HEALTHY_SESSION {
                short_sessions = short_sessions.saturating_add(1);
            } else {
                short_sessions = 0;
            }

            match outcome {
                ConnectionOutcome::ServerClosed => {
                    retry_cause = "server requested reconnect".into();
                    tracing::debug!(
                        event.name = "client.connection.server_closed",
                        "server emitted connection-closing frame; reconnect cycle reset",
                    );
                    retry_counter = 0;
                    refreshed_for_current_attempt = false;
                }
                ConnectionOutcome::Cancelled => break,
                ConnectionOutcome::HttpStatus {
                    status,
                    body,
                    request_id,
                    retry_after,
                } => match status {
                    401 if auth.is_some() && !refreshed_for_current_attempt => {
                        let outcome = state.transition(WatchEvent::AuthRejected);
                        apply_outcome(&mut last_reconnect_policy, outcome);
                    }
                    401 if refreshed_for_current_attempt => {
                        let outcome =
                            state.transition(WatchEvent::AuthRefreshCompleted { success: false });
                        apply_outcome(&mut last_reconnect_policy, outcome);
                        let _ = send_or_cancel(
                            &tx,
                            Err(ClientError::Auth(
                                "authentication rejected after refresh".into(),
                            )),
                            &mut cancel,
                            &mut parent_cancel,
                        )
                        .await;
                        break;
                    }
                    401 => {
                        let fatal =
                            state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                                "401 with no auth provider configured".to_string(),
                            )));
                        apply_outcome(&mut last_reconnect_policy, fatal);
                        let _ = send_or_cancel(
                            &tx,
                            Err(ClientError::Http {
                                status: 401,
                                body,
                                request_id,
                            }),
                            &mut cancel,
                            &mut parent_cancel,
                        )
                        .await;
                        break;
                    }
                    429 | 503 => {
                        retry_cause = format!("HTTP {status}");
                        retry_after_override = retry_after;
                        let lost = state.transition(WatchEvent::ConnectionLost {
                            reason: ConnectionLossReason::TransportError,
                        });
                        apply_outcome(&mut last_reconnect_policy, lost);
                        retry_counter = retry_counter.saturating_add(1);
                        refreshed_for_current_attempt = false;
                    }
                    s if (500..=599).contains(&s) => {
                        retry_cause = format!("HTTP {s}");
                        let lost = state.transition(WatchEvent::ConnectionLost {
                            reason: ConnectionLossReason::TransportError,
                        });
                        apply_outcome(&mut last_reconnect_policy, lost);
                        retry_counter = retry_counter.saturating_add(1);
                        refreshed_for_current_attempt = false;
                    }
                    s => {
                        let fatal = state.transition(WatchEvent::Fatal(
                            FatalKind::ProtocolViolation(format!("server returned {s}")),
                        ));
                        apply_outcome(&mut last_reconnect_policy, fatal);
                        let _ = send_or_cancel(
                            &tx,
                            Err(ClientError::Http {
                                status: s,
                                body,
                                request_id,
                            }),
                            &mut cancel,
                            &mut parent_cancel,
                        )
                        .await;
                        break;
                    }
                },
                ConnectionOutcome::TransportError(e) => {
                    retry_cause = "transport error (DNS, TCP or TLS)".into();
                    let lost = state.transition(WatchEvent::ConnectionLost {
                        reason: ConnectionLossReason::TransportError,
                    });
                    apply_outcome(&mut last_reconnect_policy, lost);
                    tracing::debug!(
                        event.name = "client.connection.lost",
                        reason = "transport_error",
                        error = %e.without_url(),
                        retry_attempt = retry_counter,
                        "connection lost; will reconnect with exponential backoff"
                    );
                    retry_counter = retry_counter.saturating_add(1);
                    refreshed_for_current_attempt = false;
                }
                ConnectionOutcome::UnexpectedEof => {
                    retry_cause = "unexpected EOF".into();
                    tracing::debug!(
                        event.name = "client.connection.lost",
                        reason = "unexpected_eof",
                        retry_attempt = retry_counter,
                        "connection ended without close frame; will reconnect with exponential backoff"
                    );
                    retry_counter = retry_counter.saturating_add(1);
                    refreshed_for_current_attempt = false;
                }
                ConnectionOutcome::HeartbeatStarved => {
                    retry_cause = "heartbeat timeout".into();
                    tracing::debug!(
                        event.name = "client.connection.lost",
                        reason = "heartbeat_starved",
                        retry_attempt = retry_counter,
                        "no SSE event observed within the heartbeat-starvation budget; will reconnect"
                    );
                    retry_counter = retry_counter.saturating_add(1);
                    refreshed_for_current_attempt = false;
                }
                ConnectionOutcome::Fatal(err) => {
                    let _ = send_or_cancel(&tx, Err(err), &mut cancel, &mut parent_cancel).await;
                    break;
                }
            }
        }

        if flush_cursor_on_exit
            && let (Some(pending), Some(store)) = (pending_commit.as_ref(), state_store.as_ref())
        {
            let checkpoint = Checkpoint::new(pending.sequence, Some(pending.event_id.clone()));
            match store.put(&resume_key, checkpoint).await {
                Ok(()) => {
                    tracing::debug!(
                        event.name = "client.resume.flushed_on_exit",
                        resume_key = %resume_key.as_hex(),
                        sequence = pending.sequence,
                        event_id = %pending.event_id,
                        "flushed pending commit to state store on supervisor exit",
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        event.name = "client.resume.flush_on_exit_failed",
                        resume_key = %resume_key.as_hex(),
                        sequence = pending.sequence,
                        error = %e,
                        "failed to flush pending commit on supervisor exit; the next run may redeliver this notification",
                    );
                }
            }
        }
    };
    super::opening::with_startup_budget(run, startup_timeout, readiness, error_tx).await;
}
