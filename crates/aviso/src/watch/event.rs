// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Driver events the supervisor feeds into the reducer.

use std::time::Duration;

use super::{ConnectionLossReason, FatalKind, GapReason};

/// Server-emitted close reason.
///
/// Carried by [`WatchEvent::ServerClose`]. These map to the
/// `connection-closing` SSE event's `reason` field on the wire (D2).
/// `MaxDurationReached` and `ServerShutdown` are routine; only
/// `EndOfStream` can terminate the session, and only in
/// [`super::WatchMode::ReplayOnly`] after `replay_completed`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerCloseReason {
    /// `connection-closing.reason = max_duration_reached`: the server's
    /// `connection_max_duration_sec` elapsed. Always routine;
    /// immediate reconnect, no backoff (D2).
    MaxDurationReached,

    /// `connection-closing.reason = server_shutdown`: the server is
    /// going down. Short backoff before the next reconnect (D2).
    ServerShutdown,

    /// `connection-closing.reason = end_of_stream`: in watch the
    /// reducer reconnects; in replay-only the reducer terminates if
    /// `replay_completed` was already true, otherwise it reconnects
    /// (D2 reconnect classifier).
    EndOfStream,
}

/// Driver events the supervisor feeds into the reducer.
///
/// The reducer is push-only: every state change is the result of an
/// event being applied via
/// [`super::WatchState::transition`](crate::watch::WatchState::transition).
/// The supervisor is the only producer of events; the reducer never
/// raises events on its own.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Transport just connected (handshake completed, ready to read
    /// frames).
    ConnectionEstablished,

    /// Transport dropped. The reducer moves to `Reconnecting` and asks
    /// the supervisor to schedule an exponential backoff.
    ConnectionLost {
        /// Why the connection dropped.
        reason: ConnectionLossReason,
    },

    /// Server emitted a `connection-closing` SSE event with the given
    /// reason.
    ServerClose {
        /// Server-supplied reason.
        reason: ServerCloseReason,
    },

    /// Supervisor scheduled a backoff. The reducer records the
    /// duration in [`super::ConnectionStatus::BackoffWait`]; it does
    /// not pick the duration itself.
    BackoffStarted(Duration),

    /// Backoff timer elapsed. The reducer moves
    /// `BackoffWait(_) -> Reconnecting` so the supervisor can attempt
    /// the next connect.
    BackoffElapsed,

    /// Supervisor received a 401 from the server. The reducer enters
    /// [`super::ConnectionStatus::RefreshingAuth`] and emits a
    /// [`super::WatchOutcome::RefreshAuth`] so the supervisor refreshes
    /// credentials. The reducer drives the supervisor, not the reverse.
    AuthRejected,

    /// Supervisor finished refreshing credentials.
    /// `success = true` returns the reducer to `Reconnecting`;
    /// `success = false` terminates with
    /// [`FatalKind::AuthenticationRejectedAfterRefresh`].
    AuthRefreshCompleted {
        /// Whether the refresh succeeded.
        success: bool,
    },

    /// SSE heartbeat received. Acknowledged for liveness; no state
    /// change.
    HeartbeatReceived,

    /// No SSE traffic of any kind for `max(3 * interval, interval + 30s)`
    /// since the last frame (D2). Treated the same as a transport-level
    /// connection loss: reconnect with exponential backoff.
    HeartbeatStarvation,

    /// A notification successfully decoded. The reducer does NOT
    /// advance any cursor on this event; the supervisor handles
    /// checkpoint advancement after triggers per D2.
    NotificationReceived {
        /// Sequence number from the `CloudEvent` `id`.
        sequence: u64,
    },

    /// Server emitted `replay-control` with `replay_completed`. In
    /// `Watch` mode the reducer moves `Replaying -> Live`; in
    /// `ReplayOnly` mode it flips `Replaying { replay_completed }` to
    /// `true`.
    ReplayCompleted,

    /// A gap was detected in the stream. The reducer enters
    /// [`super::ReplayPhase::GapDetected`] and emits
    /// [`super::WatchOutcome::Gap`]; the supervisor decides how to
    /// respond.
    GapDetected(GapReason),

    /// Terminal fatal trigger. The reducer enters
    /// [`super::ReplayPhase::Closed`] with `Fatal { kind }` and emits
    /// [`super::WatchOutcome::Stop`] with the same reason.
    Fatal(FatalKind),

    /// Caller asked the watch to stop cleanly. Reducer terminates with
    /// [`super::CloseReason::UserRequested`].
    Stop,
}
