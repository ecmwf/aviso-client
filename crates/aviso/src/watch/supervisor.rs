//! Watch supervisor: opens HTTP POSTs against the watch or replay
//! endpoint, parses the SSE response, decodes `CloudEvent`s, drives the
//! [`WatchState`] reducer, and forwards [`Notification`]s on a bounded
//! channel.
//!
//! [`run_supervisor`] is the outer reconnect loop driven by the watch
//! state machine's [`ReconnectPolicy`]. It carries `last_reconnect_policy`,
//! `retry_counter`, `retry_after_override`, `commit_cursor`,
//! `pending_commit`, and `refreshed_for_current_attempt` across iterations
//! so the per-iteration `run_one_connection` runner stays stateless on the
//! resilience axis. The outer loop terminates only on:
//!
//! - A reducer terminal state (`Fatal`, `Stop`, or natural end of
//!   replay-only after `replay_completed` and `end_of_stream`).
//! - A non-retryable HTTP status (`403`, `404`, `410`, other 4xx besides
//!   `401`/`429`, and any status outside the 2xx success class that the
//!   classifier does not retry).
//! - A second 401 within a single attempt cycle, surfaced as
//!   [`ClientError::Auth`] per D8.
//! - A persistent `StateStore` failure, surfaced as
//!   [`ClientError::StateStore`].
//! - A wire-level fatal (server `error` event, malformed `CloudEvent` id,
//!   unknown `connection-closing.reason`, gap detected).
//! - Cancellation: per-stream drop (the `NotificationStream` was dropped)
//!   or parent drop (the last `AvisoClient` clone was dropped).
//!
//! All other failure modes (transport errors, EOF without close frame,
//! heartbeat starvation, 429/503 with or without `Retry-After`, other
//! 5xx, and 401 in the first-cycle path) reconnect with exponential
//! backoff (or the `Retry-After` override) and do not surface to the
//! consumer.

use std::sync::Arc;

use reqwest::header::AUTHORIZATION;
use tokio::sync::{mpsc, oneshot, watch};
use url::Url;

use super::wire::{
    WireCloudEvent, WireConnectionClosing, WireConnectionEstablished, WireErrorEvent,
    WireReplayControl, WireWatchRequest,
};
use super::{
    ConnectionLossReason, ConnectionStatus, FatalKind, GapReason, ReconnectPolicy, ResumeStart,
    ServerCloseReason, WatchEvent, WatchMode, WatchOutcome, WatchRequest, WatchState,
};
use crate::auth::AuthProvider;
use crate::client::decrement_active_key;
use crate::state::{Checkpoint, ResumeKey, StateStore};
use crate::{ClientError, Notification, parse_cloudevent_id};

/// A notification already sent on the channel whose sequence and event id
/// will be persisted on the NEXT successful send. Promoted to
/// `commit_cursor` (and persisted to the state store) inside `drain_frames`
/// before each new notification leaves the supervisor.
pub(crate) struct PendingCommit {
    pub(crate) sequence: u64,
    pub(crate) event_id: String,
}

/// RAII guard that decrements the per-client `active_resume_keys`
/// refcount on supervisor exit, regardless of which exit path the
/// supervisor took (clean close, fatal error, cancellation, panic).
struct ActiveKeyGuard {
    active: Arc<std::sync::Mutex<std::collections::HashMap<ResumeKey, usize>>>,
    key: ResumeKey,
}

impl Drop for ActiveKeyGuard {
    fn drop(&mut self) {
        decrement_active_key(&self.active, &self.key);
    }
}

/// Outcome of one HTTP connection attempt.
///
/// Returned by [`run_one_connection`] and consumed by [`run_supervisor`].
/// The split between the inner connection-runner and the outer supervisor
/// is what lets the supervisor own retry counters, the last reconnect
/// policy, the auth refresh flag, and the commit cursor across iterations
/// without polluting the inner runner's signature with mutable state it
/// does not own.
pub(crate) enum ConnectionOutcome {
    /// The server emitted a `connection-closing` frame with a known
    /// reason. The reducer has already been advanced via
    /// `WatchEvent::ServerClose` inside [`drain_frames`]; the outer
    /// supervisor reads `state.is_terminal()` and the captured
    /// `last_reconnect_policy` to decide whether to reconnect.
    ServerClosed,
    /// HTTP status was non-200 on the initial response. Carries the
    /// full response context so the supervisor can either log it
    /// (retryable statuses), surface it (terminal statuses), or extract
    /// `Retry-After` (429 / 503). The reducer is NOT advanced inside
    /// [`run_one_connection`] for this outcome; the supervisor fires
    /// the appropriate `WatchEvent` based on classification.
    HttpStatus {
        /// HTTP status code from the response.
        status: u16,
        /// Verbatim response body (may be empty).
        body: String,
        /// Server-supplied `X-Request-ID`, when present.
        request_id: Option<String>,
        /// Parsed `Retry-After` header value, capped at 5 minutes.
        retry_after: Option<std::time::Duration>,
    },
    /// reqwest reported a transport error (TLS, connect, mid-stream
    /// read). The outer supervisor fires `WatchEvent::ConnectionLost {
    /// reason: TransportError }` and reconnects with exponential
    /// backoff.
    TransportError(reqwest::Error),
    /// The TCP connection closed cleanly (EOF) without a server-emitted
    /// `connection-closing` frame. The reducer has already been advanced
    /// via `WatchEvent::ConnectionLost { reason: UnexpectedEof }` inside
    /// `run_one_connection`; the outer supervisor reads this outcome and
    /// reconnects with exponential backoff. Long-lived watches survive
    /// NAT timeouts and half-open sockets via this path.
    UnexpectedEof,
    /// The heartbeat watchdog fired: no SSE event of any kind arrived
    /// within `max(3 * heartbeat_interval, heartbeat_interval + 30s)`.
    /// The reducer has already been advanced via
    /// `WatchEvent::HeartbeatStarvation` inside `run_one_connection`;
    /// the outer supervisor reads this outcome and reconnects with
    /// exponential backoff. Defends against silently-dead connections
    /// (NAT idle timeout, server-side application hang behind a healthy
    /// reverse proxy, half-open sockets after network change).
    HeartbeatStarved,
    /// The wire delivered a frame the supervisor must surface as a
    /// typed error: malformed `CloudEvent` id, server `error` event,
    /// unknown `connection-closing` reason, or gap detected. The
    /// reducer has already transitioned. The outer supervisor sends
    /// the error and exits.
    Fatal(ClientError),
    /// Cancellation observed (per-stream drop). The outer supervisor
    /// exits without surfacing anything.
    Cancelled,
}

/// Heartbeat-starvation budget per D2: `max(3 * interval, interval + 30s)`.
/// At the default 30 s interval the budget is 90 s; lower intervals
/// produce smaller budgets but with a 30 s absolute floor so transient
/// network slowness does not trip the watchdog.
///
/// Capped at `u32::MAX` seconds (about 136 years) so the supervisor's
/// `Instant::now() + budget` cannot overflow on any platform regardless
/// of the user-supplied `heartbeat_interval`. Effectively "no timeout"
/// at the cap; a `heartbeat_interval` large enough to saturate is
/// already a misconfiguration but the supervisor must not panic on it.
fn heartbeat_starvation_budget(interval: std::time::Duration) -> std::time::Duration {
    const ABSOLUTE_CAP: std::time::Duration = std::time::Duration::from_secs(u32::MAX as u64);
    let three_x = interval.saturating_mul(3);
    let plus_30 = interval.saturating_add(std::time::Duration::from_secs(30));
    three_x.max(plus_30).min(ABSOLUTE_CAP)
}

/// Apply a [`WatchOutcome`] to the supervisor's `last_reconnect_policy`
/// cache.
///
/// The reducer returns the outcome by value; this helper extracts the
/// reconnect policy (when present) and stores it for the outer loop's
/// next backoff calculation. Other outcome variants are read separately
/// by the loop via `state.connection_status()` and
/// `state.is_terminal()`.
///
/// Taking `WatchOutcome` by value (not by mutable reference to the
/// reducer) sidesteps the overlapping-borrow problem at call sites:
/// `let outcome = state.transition(...); apply_outcome(&mut policy, outcome);`
/// keeps the two `state` borrows separate.
#[allow(
    clippy::needless_pass_by_value,
    reason = "WatchOutcome is taken by value because the reducer already returns it by value; threading `&outcome` through the call sites only adds an extra borrow without avoiding any clone (the only ProtocolViolation String inside Fatal arrives already owned from the reducer and would be moved here too). The two-statement pattern `let outcome = state.transition(...); apply_outcome(&mut policy, outcome);` keeps the borrow of `state` separate from the borrow of `last_reconnect_policy`, which is the actual reason the helper exists."
)]
fn apply_outcome(last_reconnect_policy: &mut Option<ReconnectPolicy>, outcome: WatchOutcome) {
    match outcome {
        WatchOutcome::Reconnect { policy } => {
            *last_reconnect_policy = Some(policy);
        }
        WatchOutcome::Continue
        | WatchOutcome::RefreshAuth
        | WatchOutcome::Gap { .. }
        | WatchOutcome::Stop { .. } => {}
    }
}

/// Internal channel capacity for the supervisor's notification mpsc. See
/// the [`super::NotificationStream`] doc comment for the backpressure
/// contract this constant participates in.
pub(crate) const CHANNEL_CAPACITY: usize = 128;

/// Drive a watch session through any number of reconnect cycles. Owns its
/// inputs by value; the spawn caller in
/// [`crate::client::AvisoClient::watch`] passes cloned or `Arc`-shared
/// handles so the task is `'static` without forcing the supervisor to
/// keep an [`crate::AvisoClient`] alive. Holding only the bits it needs
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
) {
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
    let mut retry_after_override: Option<std::time::Duration> = None;
    let mut commit_cursor: Option<u64> = initial_cursor.as_ref().and_then(|r| match r {
        ResumeStart::AfterSequence(n) => Some(*n),
        ResumeStart::Date(_) => None,
    });
    let mut pending_commit: Option<PendingCommit> = None;
    let mut refreshed_for_current_attempt: bool = false;

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
            let attempt = retry_counter.saturating_sub(1);
            let delay = retry_after_override
                .take()
                .unwrap_or_else(|| super::backoff::compute_backoff(attempt, policy));
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
                let outcome = state.transition(WatchEvent::AuthRefreshCompleted { success: false });
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
        //   2. The last-sent-but-not-yet-committed sequence in
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
        )
        .await;

        match outcome {
            ConnectionOutcome::ServerClosed => {
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
                    let fatal = state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
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
                    retry_after_override = retry_after;
                    let lost = state.transition(WatchEvent::ConnectionLost {
                        reason: ConnectionLossReason::TransportError,
                    });
                    apply_outcome(&mut last_reconnect_policy, lost);
                    retry_counter = retry_counter.saturating_add(1);
                    refreshed_for_current_attempt = false;
                }
                s if (500..=599).contains(&s) => {
                    let lost = state.transition(WatchEvent::ConnectionLost {
                        reason: ConnectionLossReason::TransportError,
                    });
                    apply_outcome(&mut last_reconnect_policy, lost);
                    retry_counter = retry_counter.saturating_add(1);
                    refreshed_for_current_attempt = false;
                }
                s => {
                    let fatal = state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                        format!("server returned {s}"),
                    )));
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
            ConnectionOutcome::TransportError(_e) => {
                let lost = state.transition(WatchEvent::ConnectionLost {
                    reason: ConnectionLossReason::TransportError,
                });
                apply_outcome(&mut last_reconnect_policy, lost);
                retry_counter = retry_counter.saturating_add(1);
                refreshed_for_current_attempt = false;
            }
            ConnectionOutcome::UnexpectedEof | ConnectionOutcome::HeartbeatStarved => {
                retry_counter = retry_counter.saturating_add(1);
                refreshed_for_current_attempt = false;
            }
            ConnectionOutcome::Fatal(err) => {
                let _ = send_or_cancel(&tx, Err(err), &mut cancel, &mut parent_cancel).await;
                break;
            }
        }
    }
}

/// Drive one HTTP connection: open the request, classify the initial
/// response, decode chunks, feed them through the SSE parser plus
/// [`drain_frames`], and return a [`ConnectionOutcome`] the outer
/// supervisor loop dispatches on.
///
/// Returns:
/// - `ServerClosed` when the wire delivered a recognised
///   `connection-closing` frame; the reducer has transitioned via
///   `WatchEvent::ServerClose` and the outer loop reads the resulting
///   state to decide whether to reconnect or terminate.
/// - `HttpStatus { status, body, request_id, retry_after }` when the
///   initial response was not exactly `200 OK`. The reducer is NOT
///   advanced here; the outer loop classifies the status.
/// - `TransportError(_)` on a reqwest-level error (TLS, connect,
///   mid-stream read).
/// - `UnexpectedEof` when the response body ended without a
///   `connection-closing` frame; the reducer has transitioned via
///   `WatchEvent::ConnectionLost { reason: UnexpectedEof }`.
/// - `HeartbeatStarved` when the per-chunk `timeout(budget, ...)`
///   fired; the reducer has transitioned via
///   `WatchEvent::HeartbeatStarvation`.
/// - `Fatal(ClientError)` for terminal wire-level conditions surfaced
///   by [`drain_frames`] (malformed `CloudEvent` id, server `error`
///   event, unknown `connection-closing.reason`, decode failure,
///   gap detected, state-store failure).
/// - `Cancelled` when per-stream drop or parent drop fired during any
///   await.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the supervisor's collaborators are intentionally passed by reference rather than bundled into a context struct, so each await point owns clear borrows; the function's length is the natural shape of a single-connection runner that maps every initial-response and chunk-loop branch to a `ConnectionOutcome`, and splitting it further obscures the mapping table; threading triggers and trigger_states as additional references keeps the same shape and the cancel-safety analysis local to each await point"
)]
async fn run_one_connection(
    state: &mut WatchState,
    last_reconnect_policy: &mut Option<ReconnectPolicy>,
    request: &WatchRequest,
    wire_from: Option<&ResumeStart>,
    commit_cursor: &mut Option<u64>,
    pending_commit: &mut Option<PendingCommit>,
    state_store: Option<&Arc<dyn StateStore>>,
    resume_key: &ResumeKey,
    http: &reqwest::Client,
    base_url: &Url,
    auth: Option<&Arc<dyn AuthProvider>>,
    heartbeat_interval: std::time::Duration,
    retry_counter: &mut u32,
    trigger_states: &mut [crate::watch::trigger::TriggerState],
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
) -> ConnectionOutcome {
    let budget = heartbeat_starvation_budget(heartbeat_interval);
    let endpoint = match request.mode() {
        WatchMode::Watch => "api/v1/watch",
        WatchMode::ReplayOnly => "api/v1/replay",
    };
    let url = match base_url.join(endpoint) {
        Ok(u) => u,
        Err(e) => {
            return ConnectionOutcome::Fatal(ClientError::Config(format!(
                "build watch endpoint url from base {base_url} and path {endpoint:?}: {e}"
            )));
        }
    };
    let body = match WireWatchRequest::from_parts(request.event_type(), request.filter(), wire_from)
    {
        Ok(b) => b,
        Err(e) => return ConnectionOutcome::Fatal(e),
    };

    let auth_header = match auth {
        Some(provider) => {
            let result = tokio::select! {
                biased;
                _ = parent_cancel.changed() => return ConnectionOutcome::Cancelled,
                _ = &mut *cancel => return ConnectionOutcome::Cancelled,
                v = provider.authorization_header() => v,
            };
            match result {
                Ok(v) => Some(v),
                Err(e) => return ConnectionOutcome::Fatal(e),
            }
        }
        None => None,
    };

    let mut builder = http.post(url).json(&body);
    if let Some(value) = auth_header {
        builder = builder.header(AUTHORIZATION, value);
    }

    let send_result = tokio::select! {
        biased;
        _ = parent_cancel.changed() => return ConnectionOutcome::Cancelled,
        _ = &mut *cancel => return ConnectionOutcome::Cancelled,
        r = builder.send() => r,
    };
    let mut response = match send_result {
        Ok(r) => r,
        Err(e) => return ConnectionOutcome::TransportError(e),
    };

    // SSE streams MUST return 200. A 2xx-but-not-200 (204 No Content, 206
    // Partial Content, etc.) would have no streamable body and the
    // supervisor would EOF immediately into a reconnect loop without ever
    // surfacing the misclassified status to the consumer. Require exact
    // 200 so unexpected 2xx variants surface as `HttpStatus` with the
    // verbatim status code and the classifier in the outer loop decides
    // whether to retry or fatal them.
    if response.status() != reqwest::StatusCode::OK {
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .map(String::from);
        let retry_after =
            super::retry_after::parse_retry_after(response.headers().get("retry-after"));
        let body_result = tokio::select! {
            biased;
            _ = parent_cancel.changed() => return ConnectionOutcome::Cancelled,
            _ = &mut *cancel => return ConnectionOutcome::Cancelled,
            b = response.bytes() => b,
        };
        let body_bytes = match body_result {
            Ok(b) => b,
            Err(e) => return ConnectionOutcome::TransportError(e),
        };
        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        return ConnectionOutcome::HttpStatus {
            status,
            body,
            request_id,
            retry_after,
        };
    }

    let connected = state.transition(WatchEvent::ConnectionEstablished);
    apply_outcome(last_reconnect_policy, connected);
    // Reset the retry counter as soon as the HTTP handshake succeeds.
    // Subsequent mid-stream failures (UnexpectedEof, HeartbeatStarved,
    // mid-stream TransportError) start a fresh failures-since-last-
    // success streak rather than inflating onto whatever streak led to
    // the just-completed connect. `ServerClosed` separately resets too,
    // but that path only covers server-emitted close frames; this
    // resets even when the session ends ungracefully later.
    *retry_counter = 0;

    let mut parser = finesse::Parser::new();
    let mut gap_guard = GapGuard::starting_from(wire_from);

    loop {
        // The heartbeat budget measures time waiting for the next wire
        // chunk, not the supervisor's total iteration time. Wrapping
        // each `response.chunk().await` in a fresh `timeout(budget, ...)`
        // means time spent in `drain_frames` (state-store `put`, channel
        // backpressure inside `send_or_cancel`, parser work) does NOT
        // consume the budget. Otherwise a slow consumer that filled the
        // bounded channel could provoke a false `HeartbeatStarved`
        // reconnect even with a healthy server.
        let timed = tokio::select! {
            biased;
            _ = parent_cancel.changed() => {
                let stop = state.transition(WatchEvent::Stop);
                apply_outcome(last_reconnect_policy, stop);
                return ConnectionOutcome::Cancelled;
            }
            _ = &mut *cancel => {
                let stop = state.transition(WatchEvent::Stop);
                apply_outcome(last_reconnect_policy, stop);
                return ConnectionOutcome::Cancelled;
            }
            r = tokio::time::timeout(budget, response.chunk()) => r,
        };
        let chunk = match timed {
            Ok(c) => c,
            Err(_elapsed) => {
                let starved = state.transition(WatchEvent::HeartbeatStarvation);
                apply_outcome(last_reconnect_policy, starved);
                return ConnectionOutcome::HeartbeatStarved;
            }
        };
        let eof = matches!(chunk, Ok(None));
        match chunk {
            Ok(Some(bytes)) => parser.feed(&bytes),
            Ok(None) => parser.end(),
            Err(transport_e) => return ConnectionOutcome::TransportError(transport_e),
        }
        match drain_frames(
            &mut parser,
            state,
            last_reconnect_policy,
            &mut gap_guard,
            commit_cursor,
            pending_commit,
            state_store,
            resume_key,
            request.triggers(),
            trigger_states,
            tx,
            cancel,
            parent_cancel,
        )
        .await
        {
            Ok(DrainOutcome::Continue) => {}
            Ok(DrainOutcome::ServerClosed) => return ConnectionOutcome::ServerClosed,
            Ok(DrainOutcome::StopRequested) => return ConnectionOutcome::Cancelled,
            Err(terminal_err) => return ConnectionOutcome::Fatal(terminal_err),
        }
        if state.is_terminal() {
            return ConnectionOutcome::ServerClosed;
        }
        if eof {
            // The wire ended without a `connection-closing` frame. The
            // outer reconnect loop classifies this as routine transport
            // loss and reconnects with exponential backoff; long-lived
            // watches survive NAT timeouts and half-open sockets through
            // this path.
            let lost = state.transition(WatchEvent::ConnectionLost {
                reason: ConnectionLossReason::UnexpectedEof,
            });
            apply_outcome(last_reconnect_policy, lost);
            return ConnectionOutcome::UnexpectedEof;
        }
    }
}

/// Result of one call to [`drain_frames`].
///
/// `ServerClosed` is the supervisor's signal that a known
/// `connection-closing` frame was observed; in this single-connection
/// version of the supervisor, that always means "exit cleanly". A future
/// reconnect-loop revision will replace this with a richer signal that
/// carries the close reason.
///
/// `StopRequested` carries the "consumer is gone or cancellation fired"
/// signal up to [`run_one_connection`] so it does not re-poll the cancel
/// `oneshot::Receiver` after it has already resolved (which would panic).
#[derive(Debug, PartialEq, Eq)]
enum DrainOutcome {
    Continue,
    ServerClosed,
    StopRequested,
}

/// Drain every frame currently queued in the SSE parser and feed it
/// through the supervisor's mapping table.
///
/// Returns `Ok(DrainOutcome)`:
/// - `Continue`: routine frame handling; caller keeps reading from the
///   wire.
/// - `ServerClosed`: a `connection-closing` frame was processed; the
///   reducer's `WatchEvent::ServerClose` transition has run and the
///   caller should stop reading from this connection.
/// - `StopRequested`: per-stream or parent-drop cancellation observed
///   during a `send_or_cancel`, or the consumer dropped the receiver;
///   the caller should exit without surfacing anything.
///
/// Returns `Err(ClientError)` only for terminal wire-level conditions
/// that the consumer must see as a typed error item on the stream: a
/// malformed `CloudEvent` id, a server `error` event, an unknown
/// `connection-closing.reason`, a JSON decode failure, or a
/// state-store persistence failure.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the SSE-event-type dispatch is a flat match over six wire variants; splitting each branch into its own helper trades readability for line count, and reviewers want to see the full mapping table in one place"
)]
async fn drain_frames(
    parser: &mut finesse::Parser,
    state: &mut WatchState,
    last_reconnect_policy: &mut Option<ReconnectPolicy>,
    gap_guard: &mut GapGuard,
    commit_cursor: &mut Option<u64>,
    pending_commit: &mut Option<PendingCommit>,
    state_store: Option<&Arc<dyn StateStore>>,
    resume_key: &ResumeKey,
    triggers: &[crate::watch::Trigger],
    trigger_states: &mut [crate::watch::trigger::TriggerState],
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
) -> Result<DrainOutcome, ClientError> {
    while let Some(frame) = parser.next_frame() {
        let finesse::Frame::Message(msg) = frame else {
            continue;
        };
        match msg.event.as_str() {
            "live-notification" | "replay" => {
                let raw: serde_json::Value = serde_json::from_str(&msg.data)?;
                let top_type = raw.get("type").and_then(|v| v.as_str());
                if top_type == Some("connection_established") {
                    let _: WireConnectionEstablished = serde_json::from_value(raw)?;
                    apply_outcome(
                        last_reconnect_policy,
                        state.transition(WatchEvent::ConnectionEstablished),
                    );
                    continue;
                }
                let wire: WireCloudEvent = serde_json::from_value(raw)?;
                let (event_type, sequence) = match parse_cloudevent_id(&wire.id) {
                    Ok(v) => v,
                    Err(e) => {
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::Fatal(FatalKind::MalformedEvent)),
                        );
                        return Err(e);
                    }
                };
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::NotificationReceived { sequence }),
                );
                match gap_guard.observe(sequence) {
                    Ok(()) => {
                        let notification = Notification {
                            event_type: event_type.clone(),
                            sequence,
                            identifier: wire.data.identifier,
                            payload: wire.data.payload,
                            request_id: None,
                        };
                        // Commit-on-next-send: persist the *previous* notification
                        // before sending the current one, so pulling N implies the
                        // previous send (item N-1) is durable. Always advance the
                        // in-memory `commit_cursor` to keep reconnect-from-current
                        // working without a store. If a store is configured and
                        // its `put` fails, surface a terminal error.
                        if let Some(prev) = pending_commit.as_ref() {
                            if let Some(store) = state_store {
                                let checkpoint =
                                    Checkpoint::new(prev.sequence, Some(prev.event_id.clone()));
                                // CANCELLATION SAFETY: this `put` is
                                // INTENTIONALLY NOT raced against cancel
                                // arms. The supervisor's contract is "an
                                // in-progress store write is allowed to
                                // complete so the underlying file (or
                                // future durable backend) is never left
                                // half-written". Cancellation that
                                // arrives DURING the put extends exit
                                // latency by the put's duration; the
                                // next `send_or_cancel` then observes
                                // the cancel and the supervisor exits.
                                // Cancellation that arrives between the
                                // preceding chunk-read and this put is
                                // NOT observed here (there is no cancel
                                // check between chunk decoding and this
                                // put); the put runs anyway, which is
                                // harmless because puts are idempotent
                                // and the next process resumes from
                                // exactly the same cursor. Cancellation
                                // that arrived earlier was observed at
                                // the preceding chunk-read select and
                                // this code path is never reached.
                                // The latency trade-off (typically tens
                                // of milliseconds for `JsonFileStore`'s
                                // `fsync` on local disk) is documented
                                // on the `StateStore` trait so custom
                                // implementations know to keep `put`
                                // bounded.
                                if let Err(e) = store.put(resume_key, checkpoint).await {
                                    return Err(ClientError::from(e));
                                }
                            }
                            *commit_cursor = Some(prev.sequence);
                        }
                        // Trigger pipeline runs BEFORE the channel send so a
                        // required-trigger failure terminates the watch
                        // without ever advancing `pending_commit` for this
                        // notification; the cursor stays at the previous
                        // value and restart re-delivers this N. The pipeline
                        // itself is cancel-aware between triggers and
                        // between retry backoffs but lets a single dispatch
                        // attempt run to completion (same atomicity contract
                        // as `state_store.put`).
                        match crate::watch::trigger::dispatch_triggers(
                            triggers,
                            trigger_states,
                            &notification,
                            parent_cancel,
                            cancel,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(crate::watch::trigger::DispatchOutcome::Cancelled) => {
                                return Ok(DrainOutcome::StopRequested);
                            }
                            Err(crate::watch::trigger::DispatchOutcome::RequiredFailed {
                                kind,
                                source,
                            }) => {
                                let err = ClientError::TriggerFailed { kind, source };
                                let _ = send_or_cancel(tx, Err(err), cancel, parent_cancel).await;
                                return Ok(DrainOutcome::StopRequested);
                            }
                        }
                        if send_or_cancel(tx, Ok(notification), cancel, parent_cancel)
                            .await
                            .is_err()
                        {
                            return Ok(DrainOutcome::StopRequested);
                        }
                        *pending_commit = Some(PendingCommit {
                            sequence,
                            event_id: format!("{event_type}@{sequence}"),
                        });
                    }
                    Err(reason) => {
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::GapDetected(reason)),
                        );
                        let _ = send_or_cancel(
                            tx,
                            Err(ClientError::HistoryGap { reason }),
                            cancel,
                            parent_cancel,
                        )
                        .await;
                        return Ok(DrainOutcome::StopRequested);
                    }
                }
            }
            "heartbeat" => {
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::HeartbeatReceived),
                );
            }
            "replay-control" => {
                let wire: WireReplayControl = serde_json::from_str(&msg.data)?;
                match wire.tag.as_str() {
                    "replay_completed" => {
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::ReplayCompleted),
                        );
                    }
                    "notification_replay_limit_reached" => {
                        let Some(max_allowed) = wire.max_allowed else {
                            // The server's payload for this control event is
                            // documented to carry `max_allowed`. Silently
                            // defaulting a missing value would publish a
                            // misleading `ReplayLimitReached { max_allowed: 0 }`
                            // and hide a server protocol regression; surface a
                            // typed protocol error instead.
                            let message =
                                "replay-control: notification_replay_limit_reached missing max_allowed"
                                    .to_string();
                            apply_outcome(
                                last_reconnect_policy,
                                state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                                    message.clone(),
                                ))),
                            );
                            return Err(ClientError::StreamProtocol {
                                message,
                                request_id: None,
                            });
                        };
                        let reason = GapReason::ReplayLimitReached { max_allowed };
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::GapDetected(reason)),
                        );
                        let _ = send_or_cancel(
                            tx,
                            Err(ClientError::HistoryGap { reason }),
                            cancel,
                            parent_cancel,
                        )
                        .await;
                        return Ok(DrainOutcome::StopRequested);
                    }
                    _other => {}
                }
            }
            "connection-closing" => {
                let wire: WireConnectionClosing = serde_json::from_str(&msg.data)?;
                let reason = match wire.reason.as_str() {
                    "server_shutdown" => ServerCloseReason::ServerShutdown,
                    "max_duration_reached" => ServerCloseReason::MaxDurationReached,
                    "end_of_stream" => ServerCloseReason::EndOfStream,
                    other => {
                        let message = format!("unknown connection-closing reason: {other}");
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                                message.clone(),
                            ))),
                        );
                        return Err(ClientError::StreamProtocol {
                            message,
                            request_id: wire.request_id,
                        });
                    }
                };
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::ServerClose { reason }),
                );
                // Every recognised `connection-closing` reason ends this
                // connection. The outer reconnect loop reads the reducer's
                // post-transition state to decide whether to reconnect; in
                // watch mode the routine close reasons all reconnect, in
                // replay-only mode `end_of_stream` after `replay_completed`
                // terminates naturally.
                return Ok(DrainOutcome::ServerClosed);
            }
            "error" => {
                let wire: WireErrorEvent = serde_json::from_str(&msg.data)?;
                let message = wire
                    .message
                    .clone()
                    .or_else(|| wire.error.clone())
                    .unwrap_or_else(|| "server error event".to_string());
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                        message.clone(),
                    ))),
                );
                return Err(ClientError::StreamProtocol {
                    message,
                    request_id: wire.request_id,
                });
            }
            _other => {
                // Unknown event type: silently ignored in this iteration; a
                // future commit may add a TRACE log or a strict-mode
                // termination. Reducer state unchanged.
            }
        }
    }
    Ok(DrainOutcome::Continue)
}

/// Send `msg` on `tx`, racing with both cancellation sources.
///
/// Returns `Ok(())` on successful send. Returns `Err(())` when any of these
/// fires first: the consumer dropped the receiver (`mpsc::SendError`), the
/// per-stream cancellation oneshot fired, or the parent-drop watch channel
/// flipped. All three cases mean "stop draining and exit cleanly".
///
/// Racing the parent-drop arm here matters when the channel is full: if a
/// supervisor is parked on `tx.send().await` because a slow consumer let
/// the bounded mpsc fill, dropping the parent `AvisoClient` must still
/// terminate the supervisor within one event-loop tick.
async fn send_or_cancel(
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    msg: Result<Notification, ClientError>,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
) -> Result<(), ()> {
    tokio::select! {
        biased;
        _ = parent_cancel.changed() => Err(()),
        _ = &mut *cancel => Err(()),
        result = tx.send(msg) => result.map_err(|_| ()),
    }
}

/// Sequence-jump detector. Holds the next expected sequence; updates on
/// every observation; reports a gap when the observed sequence is strictly
/// greater than expected.
///
/// Sequence going backwards (observed less than expected) is treated as a
/// duplicate or server-side anomaly and ignored at this layer; a future
/// commit may surface it as a structured WARN log.
struct GapGuard {
    expected: Option<u64>,
}

impl GapGuard {
    fn starting_from(from: Option<&ResumeStart>) -> Self {
        let expected = match from {
            Some(ResumeStart::AfterSequence(n)) => n.checked_add(1),
            _ => None,
        };
        Self { expected }
    }

    fn observe(&mut self, observed: u64) -> Result<(), GapReason> {
        match self.expected {
            None => {
                self.expected = observed.checked_add(1);
                Ok(())
            }
            Some(expected) if observed == expected => {
                self.expected = observed.checked_add(1);
                Ok(())
            }
            Some(expected) if observed > expected => {
                Err(GapReason::SequenceJump { expected, observed })
            }
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "test code: panic-on-unexpected is the standard test diagnostic; small helpers move JSON values into wiremock bodies"
)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio::sync::{mpsc, oneshot};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        AuthProvider, ClientError, GapGuard, GapReason, Notification, ResumeKey, ResumeStart,
        StateStore, WatchRequest, run_supervisor, send_or_cancel,
    };
    use std::sync::Arc;

    fn sse_chunk(event_type: &str, data: serde_json::Value) -> String {
        format!("event: {event_type}\ndata: {data}\n\n")
    }

    fn cloud_event(event_type: &str, sequence: u64) -> serde_json::Value {
        json!({
            "id": format!("{event_type}@{sequence}"),
            "source": "https://aviso.example",
            "type": format!("int.ecmwf.aviso.{event_type}"),
            "time": "2026-05-17T12:34:56Z",
            "data": {
                "identifier": { "country": "UK" },
                "payload": { "n": sequence }
            }
        })
    }

    fn cloud_event_with_payload(
        event_type: &str,
        sequence: u64,
        payload: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "id": format!("{event_type}@{sequence}"),
            "type": format!("int.ecmwf.aviso.{event_type}"),
            "data": {
                "identifier": {},
                "payload": payload
            }
        })
    }

    fn closing(reason: &str) -> serde_json::Value {
        json!({
            "reason": reason,
            "timestamp": "2026-05-17T13:00:00Z",
            "message": "",
            "topic": "mars",
            "request_id": "req-close"
        })
    }

    /// Test helper that spawns a `run_supervisor` task against a `MockServer`.
    ///
    /// Returns the consumer-side mpsc receiver, the per-stream cancel
    /// sender, the supervisor `JoinHandle`, and the parent-drop `watch`
    /// sender. The fourth value is the load-bearing test fixture for the
    /// parent-cancel cascade: as long as the test holds it, the
    /// supervisor's `parent_cancel.changed()` arm stays parked. Tests
    /// that want to trigger the cascade drop it explicitly; tests that
    /// do not, simply bind it to a name and let RAII drop it at scope
    /// end (after the supervisor `JoinHandle` has already completed,
    /// so the dangling sender drop never fires the cascade against an
    /// already-exited supervisor).
    #[allow(
        clippy::type_complexity,
        reason = "test helper's return tuple aggregates the four channels the supervisor needs (notification receiver, per-stream cancel, JoinHandle, parent-drop sender); each component is named at the call site via destructuring so the complexity does not propagate"
    )]
    fn start_supervisor(
        server: &MockServer,
        request: WatchRequest,
    ) -> (
        mpsc::Receiver<Result<Notification, ClientError>>,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
        tokio::sync::watch::Sender<bool>,
    ) {
        start_supervisor_with_store(server, request, None)
    }

    /// Variant of [`start_supervisor`] that wires a state store for the
    /// supervisor's commit-on-next-send cursor advancement. Returns the
    /// same 4-tuple; the test recomputes the resume key from the server
    /// URL and event type for `store.get(...)` queries.
    #[allow(
        clippy::type_complexity,
        reason = "test helper's return tuple is unchanged from start_supervisor; only the state-store wiring differs"
    )]
    fn start_supervisor_with_store(
        server: &MockServer,
        request: WatchRequest,
        store: Option<Arc<dyn StateStore>>,
    ) -> (
        mpsc::Receiver<Result<Notification, ClientError>>,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
        tokio::sync::watch::Sender<bool>,
    ) {
        let capacity = if store.is_some() {
            1
        } else {
            super::CHANNEL_CAPACITY
        };
        let (tx, rx) = mpsc::channel(capacity);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
        let http = reqwest::Client::builder().build().unwrap();
        let no_auth: Option<Arc<dyn AuthProvider>> = None;
        let heartbeat_interval = std::time::Duration::from_secs(30);
        let resume_key = ResumeKey::new(&base_url, request.event_type(), &json!({}), None).unwrap();
        let (drop_sender, parent_cancel) = tokio::sync::watch::channel(false);
        let active_resume_keys = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
            ResumeKey,
            usize,
        >::new()));
        let handle = tokio::spawn(run_supervisor(
            request,
            http,
            base_url,
            no_auth,
            heartbeat_interval,
            store,
            resume_key,
            tx,
            cancel_rx,
            parent_cancel,
            active_resume_keys,
        ));
        (rx, cancel_tx, handle, drop_sender)
    }

    #[tokio::test]
    async fn drop_cancel_oneshot_makes_supervisor_exit_within_bounded_time() {
        // The supervisor must observe cancellation even while it is parked
        // on a bounded-channel `tx.send().await` because the consumer fell
        // behind. The test constructs that exact situation: a response body
        // carrying many more notifications than the channel capacity, no
        // terminating frame, a consumer that takes one item and then stops.
        // Once the channel fills, the supervisor blocks on `send_or_cancel`.
        // Dropping the cancel sender then fires the `select!` arm and the
        // supervisor exits cleanly within 500 ms.
        let server = MockServer::start().await;
        let notification_count = super::CHANNEL_CAPACITY + 64;
        let mut body = String::new();
        for n in 1..=notification_count {
            body.push_str(&sse_chunk(
                "live-notification",
                cloud_event("mars", n as u64),
            ));
        }
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("first notification should arrive promptly")
            .expect("channel must not close before the first item");
        assert!(
            first.is_ok(),
            "first item should be Ok(Notification): {first:?}"
        );

        drop(cancel_tx);

        tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("supervisor must exit within 500ms of cancellation")
            .expect("supervisor task panicked");

        drop(rx);
    }

    #[tokio::test]
    async fn drain_frames_mapping_covers_live_notification_with_cloudevent() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}",
            sse_chunk("live-notification", cloud_event("mars", 7)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.event_type, "mars");
        assert_eq!(first.sequence, 7);
        // In watch mode end_of_stream reconnects; drop the cancel to break
        // the reconnect loop and let the supervisor exit.
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[tokio::test]
    async fn drain_frames_mapping_treats_connection_established_marker_as_control() {
        let server = MockServer::start().await;
        let marker = json!({
            "type": "connection_established",
            "topic": "mars",
            "timestamp": "2026-05-17T12:00:00Z",
            "connection_will_close_in_seconds": 3600u64,
            "request_id": "req-est"
        });
        let body = format!(
            "{}{}{}",
            sse_chunk("live-notification", marker),
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(
            first.sequence, 1,
            "the connection_established marker must not emit a notification"
        );
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[tokio::test]
    async fn drain_frames_mapping_does_not_confuse_payload_substring_with_marker() {
        // Regression: the disambiguation must read the *top-level* `type`
        // field; a CloudEvent whose payload contains the literal string
        // `"connection_established"` must still decode as a Notification.
        let server = MockServer::start().await;
        let ev = cloud_event_with_payload(
            "mars",
            5,
            json!({ "free_text": "connection_established was emitted earlier today" }),
        );
        let body = format!(
            "{}{}",
            sse_chunk("live-notification", ev),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.event_type, "mars");
        assert_eq!(first.sequence, 5);
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[tokio::test]
    async fn drain_frames_mapping_terminates_on_error_event_with_stream_protocol_error() {
        let server = MockServer::start().await;
        let err = json!({
            "error": "stream_processing_failed",
            "message": "boom",
            "topic": "mars",
            "request_id": "req-err"
        });
        let body = sse_chunk("error", err);
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        match item {
            Err(ClientError::StreamProtocol {
                message,
                request_id,
            }) => {
                assert_eq!(message, "boom");
                assert_eq!(request_id.as_deref(), Some("req-err"));
            }
            other => panic!("expected StreamProtocol, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_terminates_on_unknown_connection_closing_reason() {
        let server = MockServer::start().await;
        let body = sse_chunk(
            "connection-closing",
            closing("future_reason_we_did_not_anticipate"),
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        assert!(
            matches!(item, Err(ClientError::StreamProtocol { .. })),
            "got {item:?}"
        );
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_handles_heartbeat_as_observation_only() {
        let server = MockServer::start().await;
        let heartbeat = json!({ "timestamp": "2026-05-17T12:00:00Z", "topic": "mars" });
        let body = format!(
            "{}{}{}",
            sse_chunk("heartbeat", heartbeat),
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[tokio::test]
    async fn drain_frames_mapping_terminates_on_replay_limit_reached_with_history_gap() {
        let server = MockServer::start().await;
        let limit = json!({
            "type": "notification_replay_limit_reached",
            "topic": "mars",
            "max_allowed": 1000u64,
            "timestamp": "2026-05-17T12:00:00Z"
        });
        let body = sse_chunk("replay-control", limit);
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        match item {
            Err(ClientError::HistoryGap {
                reason: GapReason::ReplayLimitReached { max_allowed },
            }) => {
                assert_eq!(max_allowed, 1000);
            }
            other => panic!("expected HistoryGap{{ReplayLimitReached}}, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn replay_limit_reached_without_max_allowed_surfaces_stream_protocol_error() {
        // Defending against a server protocol regression: if the
        // `notification_replay_limit_reached` payload ever drops the
        // `max_allowed` field, the client must surface a typed protocol
        // error instead of silently emitting a misleading
        // `ReplayLimitReached { max_allowed: 0 }`.
        let server = MockServer::start().await;
        let limit = json!({
            "type": "notification_replay_limit_reached",
            "topic": "mars",
            "timestamp": "2026-05-17T12:00:00Z"
        });
        let body = sse_chunk("replay-control", limit);
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        match item {
            Err(ClientError::StreamProtocol { message, .. }) => {
                assert!(
                    message.contains("max_allowed"),
                    "message should name the missing field: {message}"
                );
            }
            other => panic!("expected StreamProtocol, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn connection_closing_terminates_before_any_following_frames() {
        // Even if the server keeps writing notifications after a
        // `connection-closing` frame on the same wire (which it should not,
        // but which is exactly the bug class the supervisor must guard
        // against), the supervisor must stop at the close frame and not
        // surface the trailing notifications.
        let server = MockServer::start().await;
        let body = format!(
            "{}{}{}",
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream")),
            sse_chunk("live-notification", cloud_event("mars", 99)),
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        // Sequence 99 (after the close frame) must not arrive. We give the
        // supervisor a small window to misbehave, then drop the cancel to
        // break the reconnect loop.
        let next = tokio::time::timeout(Duration::from_millis(150), rx.recv()).await;
        match next {
            Err(_) => {
                // Timeout: nothing more arrived. The expected outcome.
            }
            Ok(Some(Ok(n))) => {
                assert_ne!(
                    n.sequence, 99,
                    "the post-close-frame notification must NOT surface"
                );
            }
            Ok(other) => panic!("unexpected receive: {other:?}"),
        }
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[tokio::test]
    async fn unexpected_eof_without_close_frame_does_not_surface_fatal_error() {
        // EOF without close-frame is reclassified as routine transport
        // loss; the supervisor reconnects rather than surfacing a terminal
        // error. The end-to-end behaviour is covered by an integration
        // test (`unexpected_eof_triggers_reconnect`); this unit test pins
        // the negative invariant: no `Err(_)` item ever surfaces.
        let server = MockServer::start().await;
        let body = sse_chunk("live-notification", cloud_event("mars", 1));
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        // After the first notification the server EOFs; the supervisor
        // reconnects. The second connection serves the same body (mock
        // does not differentiate), so the next item the consumer sees
        // would be another notification (possibly flagged by GapGuard
        // because sequence 1 is observed-before-expected, which the
        // GapGuard tolerates as a backwards-jump). We do not assert on
        // the second item's exact shape here; we only assert that no
        // `Err(StreamProtocol)` surfaces.
        let next = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        if let Ok(Some(Err(e))) = next {
            panic!("EOF must not surface as a terminal error, got: {e:?}");
        }
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[tokio::test]
    async fn drain_frames_mapping_silently_ignores_unknown_sse_event_types() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}{}",
            sse_chunk("future-event-we-do-not-know", json!({ "anything": true })),
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, cancel_tx, handle, _parent_drop) =
            start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        drop(cancel_tx);
        let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
        join_result.expect("supervisor task must not panic");
    }

    #[test]
    fn heartbeat_budget_30s_interval_returns_90s() {
        assert_eq!(
            super::heartbeat_starvation_budget(Duration::from_secs(30)),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn heartbeat_budget_10s_interval_returns_40s_via_plus_30() {
        assert_eq!(
            super::heartbeat_starvation_budget(Duration::from_secs(10)),
            Duration::from_secs(40)
        );
    }

    #[test]
    fn heartbeat_budget_1s_interval_returns_31s_via_plus_30() {
        assert_eq!(
            super::heartbeat_starvation_budget(Duration::from_secs(1)),
            Duration::from_secs(31)
        );
    }

    #[test]
    fn heartbeat_budget_zero_interval_returns_30s_floor() {
        assert_eq!(
            super::heartbeat_starvation_budget(Duration::ZERO),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn heartbeat_budget_absurd_interval_caps_at_136_years() {
        let cap = Duration::from_secs(u64::from(u32::MAX));
        assert_eq!(super::heartbeat_starvation_budget(Duration::MAX), cap);
        let near_max = Duration::from_secs(u64::MAX / 2);
        assert_eq!(super::heartbeat_starvation_budget(near_max), cap);
    }

    #[test]
    fn gap_guard_no_resume_adopts_first_observed_sequence() {
        let mut g = GapGuard::starting_from(None);
        assert!(g.observe(100).is_ok());
        assert!(g.observe(101).is_ok());
        assert!(matches!(
            g.observe(105),
            Err(GapReason::SequenceJump {
                expected: 102,
                observed: 105
            })
        ));
    }

    #[test]
    fn gap_guard_after_sequence_expects_next() {
        let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(9)));
        assert!(g.observe(10).is_ok());
        assert!(g.observe(11).is_ok());
        assert!(matches!(
            g.observe(13),
            Err(GapReason::SequenceJump {
                expected: 12,
                observed: 13
            })
        ));
    }

    #[test]
    fn gap_guard_tolerates_backwards_sequence() {
        let mut g = GapGuard::starting_from(None);
        assert!(g.observe(100).is_ok());
        assert!(
            g.observe(50).is_ok(),
            "backwards observation must not be reported as a gap"
        );
    }

    #[test]
    fn gap_guard_overflow_resets_to_fresh_start() {
        let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(u64::MAX)));
        assert!(
            g.observe(0).is_ok(),
            "overflow on construction must not poison subsequent observation"
        );
    }

    #[tokio::test]
    async fn send_or_cancel_returns_err_when_cancel_fires_first() {
        let (tx, _rx) = mpsc::channel(1);
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        drop(cancel_tx);
        let (_parent_tx, mut parent_rx) = tokio::sync::watch::channel(false);
        let result = send_or_cancel(
            &tx,
            Err(ClientError::Config("dummy".into())),
            &mut cancel_rx,
            &mut parent_rx,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_or_cancel_returns_err_when_receiver_is_dropped() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let (_cancel_tx, mut cancel_rx) = oneshot::channel();
        let (_parent_tx, mut parent_rx) = tokio::sync::watch::channel(false);
        let result = send_or_cancel(
            &tx,
            Err(ClientError::Config("dummy".into())),
            &mut cancel_rx,
            &mut parent_rx,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_or_cancel_returns_err_when_parent_cancel_fires() {
        let (tx, _rx) = mpsc::channel::<Result<Notification, ClientError>>(1);
        // Fill the channel so the next send parks.
        tx.send(Err(ClientError::Config("pad".into())))
            .await
            .unwrap();
        let (_cancel_tx, mut cancel_rx) = oneshot::channel();
        let (parent_tx, mut parent_rx) = tokio::sync::watch::channel(false);
        // tx has capacity 0 with a receiver alive but never reading; tx.send
        // would park forever. Firing parent_tx must unblock send_or_cancel
        // via its new arm. The test races the send against a parallel drop.
        let driver = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(parent_tx);
        };
        let send = send_or_cancel(
            &tx,
            Err(ClientError::Config("dummy".into())),
            &mut cancel_rx,
            &mut parent_rx,
        );
        let (result, ()) = tokio::time::timeout(Duration::from_millis(200), async {
            tokio::join!(send, driver)
        })
        .await
        .expect("send_or_cancel must observe parent-drop within 200ms");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn prev_notification_is_committed_before_current_triggers_run() {
        use crate::state::MemoryStore;
        use crate::watch::Trigger;
        use tokio::time::timeout;

        let server = MockServer::start().await;
        let body = format!(
            "{}{}",
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("live-notification", cloud_event("mars", 2)),
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let store: Arc<dyn StateStore> = Arc::new(MemoryStore::new());
        let (trigger, _counter) = Trigger::test_fail_on_call(2, 0, true);
        let request = WatchRequest::watch("mars").with_triggers(vec![trigger]);
        let (mut rx, _cancel, handle, _drop) =
            start_supervisor_with_store(&server, request, Some(store.clone()));

        let first = timeout(Duration::from_secs(5), rx.recv()).await;
        let first = first
            .expect("first notification arrives within 5s")
            .expect("channel still open")
            .expect("first notification is Ok");
        assert_eq!(first.sequence, 1);

        let second = timeout(Duration::from_secs(5), rx.recv()).await;
        let second = second
            .expect("trigger failure arrives within 5s")
            .expect("channel still open");
        match second {
            Err(ClientError::TriggerFailed { .. }) => {}
            other => panic!("expected TriggerFailed on second item, got {other:?}"),
        }
        let none = timeout(Duration::from_secs(2), rx.recv()).await;
        let none = none.expect("channel closes within 2s after terminal error");
        assert!(none.is_none());

        timeout(Duration::from_secs(2), handle)
            .await
            .expect("supervisor exits within 2s")
            .expect("supervisor task must not panic");

        let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
        let resume_key = ResumeKey::new(&base_url, "mars", &json!({}), None).unwrap();
        let checkpoint = store.get(&resume_key).await.unwrap();
        let cp = checkpoint.expect("checkpoint for N=1 should be persisted");
        assert_eq!(
            cp.last_committed_sequence, 1,
            "N=1 must have been committed before N=2 triggers ran"
        );
    }
}
