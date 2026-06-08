// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Connection-status axis of the watch state machine.

use std::time::Duration;

/// Transport-level state of the watch's SSE connection.
///
/// Quoted from D2. One of the two axes of [`super::ReplayPhase`] x
/// `ConnectionStatus`. Connection-level events modify only this axis;
/// they never advance the replay phase.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Transport is open and the supervisor is reading frames.
    Connected,

    /// Transport is currently being re-established. The reducer enters
    /// this state on connection loss, on routine server-driven closes,
    /// and after backoff elapses.
    Reconnecting,

    /// Supervisor is sleeping out a backoff window before the next
    /// reconnect attempt. The carried `Duration` is the wait the
    /// supervisor scheduled (computed from its own retry counter; the
    /// reducer does not pick the duration).
    BackoffWait(Duration),

    /// Supervisor is refreshing credentials before the next reconnect.
    /// Entered on [`super::WatchEvent::AuthRejected`].
    RefreshingAuth,
}

/// Why a connection was lost.
///
/// Used as the payload of [`super::WatchEvent::ConnectionLost`]. Both
/// variants drive the same reducer behaviour (exponential backoff per
/// D2); the distinction is preserved so the supervisor can attach
/// distinct telemetry.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionLossReason {
    /// A reqwest/IO-level error: connect failure, TLS error, read
    /// timeout, peer reset, and so on.
    TransportError,

    /// Connection terminated by an unexpected end-of-file, without a
    /// server-emitted `connection-closing` event. The supervisor
    /// usually reads this as "the network went dark" or "the server
    /// process died without saying so".
    UnexpectedEof,
}
