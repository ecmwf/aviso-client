//! Watch state machine.
//!
//! This module contains the pure-logic state machine that drives a
//! watch session, as specified by ADRs D2 and D15. The state is the
//! orthogonal product of two axes,
//! [`ReplayPhase`] x [`ConnectionStatus`], and is advanced through a
//! single reducer. The reducer owns no I/O, no async runtime, and no
//! checkpoint state; the supervisor (a follow-up PR) wires it to the
//! HTTP/SSE transport, the auth provider, and the [`crate::state`]
//! store.
//!
//! In this PR only the types and the reducer ship. There is no
//! `AvisoClient::watch()` method yet; the public surface is
//! deliberately accessible only as `aviso::watch::*` until the watch
//! API shape (Stream-based versus callback-based, an open question
//! tracked in `plans/v0.3.md`) is settled. The reducer is the
//! foundation either choice builds on.
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
