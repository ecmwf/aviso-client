//! Replay-phase and supporting enums.
//!
//! The replay phase is one of the two axes of [`super::WatchMode`]-aware
//! watch state, alongside [`super::ConnectionStatus`]. It tracks where
//! the stream is in its replay-or-live lifecycle (D2, D15).

/// Initial resume position the watch was started from.
///
/// The reducer carries the start position inside [`ReplayPhase::Replaying`]
/// so it can be logged or inspected during a replay. Note the variants:
/// the reducer has no `Head` / `Live` variant because a live-only watch
/// starts directly in [`ReplayPhase::Live`] (no replay to do).
///
/// D17 caveat: a `Date` start is opaque to the reducer. The supervisor
/// converts it to a sequence cursor after the first committed
/// notification; the reducer does not own checkpoint state.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResumeStart {
    /// Resume *after* the given sequence: the supervisor sends
    /// `from_id = sequence + 1` on the wire. The variant name mirrors
    /// the semantic ("we already have everything up to and including
    /// `sequence`, give us the next event") and avoids ambiguity with
    /// the wire-level `from_id` parameter.
    AfterSequence(u64),

    /// Bootstrap from a user-supplied date string. The reducer stores
    /// it verbatim; the supervisor sends `from_date` on the wire and
    /// transitions to sequence-based resume after the first commit
    /// (D17).
    Date(String),
}

/// Replay-or-live phase of the watch.
///
/// Quoted directly from D2:
/// `Replaying { start, replay_completed: false }` is the initial phase
/// when constructed with a resume position; `Live` is the initial phase
/// when constructed without one. The reducer transitions to `Live`
/// only on receipt of a `replay_completed` event in `Watch` mode; in
/// `ReplayOnly` mode the phase stays in `Replaying` and flips
/// `replay_completed` to `true`. `GapDetected` records a gap reason;
/// `Closed` is terminal.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayPhase {
    /// Replaying backlog. `replay_completed` flips to `true` when the
    /// server emits `replay-control { replay_completed }`. In `Watch`
    /// mode the phase then moves to `Live`; in `ReplayOnly` mode it
    /// stays here until `end_of_stream`.
    Replaying {
        /// The resume position the session was started from.
        start: ResumeStart,
        /// `true` once the server's `replay_completed` control event
        /// has been received.
        replay_completed: bool,
    },

    /// Streaming live notifications. Reachable in `Watch` mode after
    /// `Replaying` completes, or directly at construction when no
    /// resume position was supplied.
    Live,

    /// A gap was detected in the stream. The supervisor surfaces this
    /// to the operator via [`super::WatchOutcome::Gap`] and decides
    /// whether to continue or terminate; the reducer never escapes this
    /// phase on its own.
    GapDetected {
        /// Reason for the gap.
        reason: GapReason,
    },

    /// Terminal phase. The reducer stays here for the rest of the
    /// session.
    Closed {
        /// Reason the session ended.
        reason: CloseReason,
    },
}

/// Why a gap was detected in the stream.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GapReason {
    /// The server reported `notification_replay_limit_reached` (D2):
    /// the client asked to replay more notifications than the server's
    /// configured cap, so some of the requested backlog will not be
    /// delivered.
    ReplayLimitReached {
        /// Server-side cap on how many notifications a single replay may
        /// return (the `max_allowed` field from the server payload).
        max_allowed: u64,
    },

    /// The wire delivered a non-consecutive sequence number
    /// mid-stream.
    SequenceJump {
        /// Sequence the client expected next.
        expected: u64,
        /// Sequence actually received.
        observed: u64,
    },
}

/// Why the watch session closed terminally.
///
/// The reducer enters [`ReplayPhase::Closed`] with one of these
/// reasons. `max_duration_reached` and `server_shutdown` are NOT close
/// reasons; per D2 they are routine reconnect triggers that mutate
/// only [`super::ConnectionStatus`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// Replay-only session ended naturally on the server's
    /// `end_of_stream` after `replay_completed` (D2).
    EndOfStream,

    /// Non-recoverable error; [`FatalKind`] carries the kind.
    Fatal {
        /// Specific fatal kind.
        kind: FatalKind,
    },

    /// The caller asked the watch to stop (`WatchEvent::Stop`).
    UserRequested,
}

/// Specific fatal-error kinds the watch can terminate with.
///
/// Each variant is reachable through [`super::WatchEvent::Fatal`] (or,
/// for `AuthenticationRejectedAfterRefresh`, through
/// [`super::WatchEvent::AuthRefreshCompleted`] with `success: false`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FatalKind {
    /// A `CloudEvent` `id` field could not be parsed per D9. Terminal
    /// to avoid livelock on a poisoned server stream.
    MalformedEvent,

    /// The server's schema fingerprint for the subscribed event type
    /// changed mid-stream. Terminal because the resume key embeds the
    /// fingerprint (D3); a continued session would silently route to
    /// the wrong checkpoint.
    SchemaFingerprintChanged,

    /// A 401 was still returned after a credential refresh. The
    /// supervisor has nothing left to try.
    AuthenticationRejectedAfterRefresh,

    /// Transport-level retries hit the cap (default 30 s exponential
    /// backoff per D2 without success).
    TransportRetriesExhausted,

    /// A wire-level protocol violation the supervisor cannot recover
    /// from (unknown SSE event type, malformed `CloudEvent` body
    /// outside of a malformed `id`, unsolicited status code, and so
    /// on). The string carries a human-readable description for
    /// logging only; the reducer does not interpret it.
    ProtocolViolation(String),
}
