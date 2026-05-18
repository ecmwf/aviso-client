//! Watch state machine.
//!
//! This module contains the pure-logic state machine that drives a
//! watch session, as specified by ADRs D2 and D15. The state is the
//! orthogonal product of two axes,
//! [`ReplayPhase`] x [`ConnectionStatus`], and is advanced through a
//! single reducer. The reducer owns no I/O, no async runtime, and no
//! checkpoint state; downstream watch supervisors wire it to the
//! HTTP/SSE transport, the auth provider, and the [`crate::state`]
//! store.
//!
//! Only the types and the reducer live in this module; there is no
//! `AvisoClient::watch()` method. The public surface is accessible
//! as `aviso::watch::*`. Higher-level watch APIs (Stream-based or
//! callback-based) build on top of this reducer.
//!
//! # Concepts
//!
//! - [`ReplayPhase`]: where the stream is in its replay-or-live
//!   lifecycle. Starts in `Replaying { start, replay_completed: false }`
//!   when constructed with a resume position, or in `Live` when
//!   constructed without one.
//! - [`ConnectionStatus`]: the transport-level state of the SSE
//!   connection (connected, reconnecting, waiting on backoff, refreshing
//!   auth).
//! - [`WatchMode`]: whether the session is `Watch` (historical-then-live;
//!   reconnects on `end_of_stream`) or `ReplayOnly` (terminates on
//!   `end_of_stream` once the server's `replay_completed` event has been
//!   received).
//! - [`WatchEvent`]: the input vocabulary the supervisor uses to drive
//!   the reducer.
//! - [`WatchOutcome`]: the reducer's reply, telling the supervisor what
//!   to do next (continue, reconnect with a [`ReconnectPolicy`], refresh
//!   auth, surface a gap, or stop).
//!
//! # Transition summary
//!
//! The full transition table (one row per event x precondition pair)
//! lives in the commit message that introduced
//! `crates/aviso/src/watch/state.rs`. The summary below is the
//! reader-friendly view; the row numbers in `state.rs`'s match arms
//! refer to the same table.
//!
//! - `ConnectionEstablished`: `connection_status` -> `Connected`.
//! - `ConnectionLost { .. }` or `HeartbeatStarvation`: reconnect with
//!   [`ReconnectPolicy::ExponentialBackoff`].
//! - `ServerClose { reason }`: dispatches on the reason.
//!   `MaxDurationReached` reconnects immediately;
//!   `ServerShutdown` applies a short backoff;
//!   `EndOfStream` in [`WatchMode::Watch`] reconnects immediately;
//!   `EndOfStream` in [`WatchMode::ReplayOnly`] terminates iff
//!   `replay_completed` was already true, otherwise reconnects.
//! - `BackoffStarted(d)` records the duration in
//!   [`ConnectionStatus::BackoffWait`]; `BackoffElapsed` returns the
//!   reducer to `Reconnecting` only if currently waiting.
//! - `AuthRejected`: `connection_status` -> `RefreshingAuth`; outcome
//!   [`WatchOutcome::RefreshAuth`].
//!   `AuthRefreshCompleted { success: true }`: `connection_status` ->
//!   `Reconnecting`.
//!   `AuthRefreshCompleted { success: false }`: terminate with
//!   [`FatalKind::AuthenticationRejectedAfterRefresh`].
//! - `HeartbeatReceived`, `NotificationReceived { .. }`: pure
//!   observations, no state change.
//! - `ReplayCompleted`: in [`WatchMode::Watch`] from `Replaying` moves
//!   to `Live`. In [`WatchMode::ReplayOnly`] from `Replaying { rc:
//!   false }` flips `replay_completed` to true. Idempotent in every
//!   other non-terminal phase.
//! - `GapDetected(reason)`: phase -> `GapDetected { reason }`; outcome
//!   [`WatchOutcome::Gap`] (dedicated, not folded into `Continue`).
//! - `Fatal(kind)`: phase -> `Closed { Fatal { kind } }`; outcome
//!   `Stop`.
//! - `Stop`: phase -> `Closed { UserRequested }`; outcome `Stop`.
//! - `Closed` is sticky: every subsequent event is a no-op returning
//!   `Continue`.
//!
//! # Usage
//!
//! ```
//! use aviso::watch::{
//!     ReconnectPolicy, ServerCloseReason, WatchEvent, WatchOutcome,
//!     WatchState,
//! };
//!
//! // A live-only watch (no replay backlog) starts directly in
//! // `Live` with `Reconnecting` while the transport opens.
//! let mut state = WatchState::watch(None);
//!
//! // Supervisor establishes the transport.
//! let _ = state.transition(WatchEvent::ConnectionEstablished);
//!
//! // Server hits its `connection_max_duration_sec`. Routine close;
//! // reconnect immediately, no backoff.
//! let outcome = state.transition(WatchEvent::ServerClose {
//!     reason: ServerCloseReason::MaxDurationReached,
//! });
//! assert_eq!(
//!     outcome,
//!     WatchOutcome::Reconnect {
//!         policy: ReconnectPolicy::Immediate,
//!     }
//! );
//!
//! // User asks the watch to stop.
//! let _ = state.transition(WatchEvent::Stop);
//! assert!(state.is_terminal());
//! ```
//!
//! # Cross-references
//!
//! - D2 (`docs/src/internals/decisions.md`): reconnect-as-norm,
//!   at-least-once delivery, state-machine sketch, reconnect classifier.
//! - D15: state machine is the orthogonal product `ReplayPhase x
//!   ConnectionStatus`, single reducer, fields private.
//! - D9: a malformed `CloudEvent` id is terminal; surfaces here through
//!   [`WatchEvent::Fatal`] with [`FatalKind::MalformedEvent`].
//! - D17: `from_date` is bootstrap-only and converts to a sequence
//!   cursor after the first commit. The reducer does not own that
//!   bookkeeping; the supervisor advances its own checkpoint state in
//!   response to [`WatchEvent::NotificationReceived`].

mod connection;
mod event;
mod mode;
mod outcome;
mod phase;
mod state;

pub use connection::{ConnectionLossReason, ConnectionStatus};
pub use event::{ServerCloseReason, WatchEvent};
pub use mode::WatchMode;
pub use outcome::{ReconnectPolicy, WatchOutcome};
pub use phase::{CloseReason, FatalKind, GapReason, ReplayPhase, ResumeStart};
pub use state::WatchState;
