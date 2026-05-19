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

use super::{ReconnectPolicy, WatchOutcome};
use crate::ClientError;

/// A notification already sent on the channel whose sequence and event id
/// will be persisted on the NEXT successful send. Promoted to
/// `commit_cursor` (and persisted to the state store) inside `drain_frames`
/// before each new notification leaves the supervisor.
pub(crate) struct PendingCommit {
    pub(crate) sequence: u64,
    pub(crate) event_id: String,
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
pub(crate) fn heartbeat_starvation_budget(interval: std::time::Duration) -> std::time::Duration {
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
pub(crate) fn apply_outcome(
    last_reconnect_policy: &mut Option<ReconnectPolicy>,
    outcome: WatchOutcome,
) {
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

/// Result of one call to `drain_frames`.
///
/// `ServerClosed` reports that a known `connection-closing` frame was
/// observed for the current connection. The outer reconnect loop in
/// `run_supervisor` then consults the reducer's resulting `WatchState`
/// (already updated with the close reason via the frame handler) to
/// decide whether to reconnect, back off, or terminate; the variant
/// itself carries no further information because the reducer state is
/// the single source of truth for that decision.
///
/// `StopRequested` carries the "consumer is gone or cancellation fired"
/// signal up to `run_one_connection` so it does not re-poll the cancel
/// `oneshot::Receiver` after it has already resolved (which would panic).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DrainOutcome {
    Continue,
    ServerClosed,
    StopRequested,
}

mod connection;
mod drain;
mod guards;
mod reconnect;

pub(crate) use guards::{ActiveKeyGuard, GapGuard, send_or_cancel};
pub(crate) use reconnect::run_supervisor;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "test code: panic-on-unexpected is the standard test diagnostic; small helpers move JSON values into wiremock bodies"
)]
mod tests;
