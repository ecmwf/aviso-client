// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Reducer outputs that tell the supervisor what to do next.

use super::{CloseReason, GapReason};

/// Reconnect strategy advised by the reducer.
///
/// Maps directly to D2's reconnect classifier. The variant tells the
/// supervisor whether to backoff and, if so, with which schedule; the
/// supervisor still owns the retry counter and the actual `Duration`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReconnectPolicy {
    /// Reconnect now, no backoff. Used for routine server-driven
    /// closes (`max_duration_reached`, `end_of_stream` in watch mode,
    /// `end_of_stream` in replay-only before `replay_completed`).
    Immediate,

    /// Reconnect after a short backoff (single-digit seconds per D2).
    /// Used for `server_shutdown`.
    ShortBackoff,

    /// Reconnect after jittered exponential backoff (250 ms to 30 s
    /// cap per D2). Used for transport errors and heartbeat
    /// starvation.
    ExponentialBackoff,
}

/// The reducer's reply to a [`super::WatchEvent`].
///
/// `#[must_use]`: the design depends on the supervisor acting on the
/// outcome. Ignoring the outcome silently breaks D2's reconnect
/// classifier.
#[must_use = "the watch supervisor must act on the outcome (reconnect, refresh auth, surface a gap, or stop)"]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchOutcome {
    /// No action required from the supervisor for this event.
    Continue,

    /// Supervisor should reconnect; the `policy` tells it which
    /// backoff schedule to apply (if any).
    Reconnect {
        /// The reconnect policy classified by D2's reconnect rules.
        policy: ReconnectPolicy,
    },

    /// Supervisor should refresh credentials and then reconnect.
    /// Emitted on [`super::WatchEvent::AuthRejected`].
    RefreshAuth,

    /// A gap was detected. The supervisor must look at the reason and
    /// decide how to respond (typically: log, then either reconnect
    /// from a later cursor or [`super::WatchEvent::Stop`]).
    Gap {
        /// The gap reason copied from
        /// [`super::WatchEvent::GapDetected`].
        reason: GapReason,
    },

    /// Watch is terminating with `reason`. The supervisor tears down
    /// the transport and surfaces the close to its caller.
    Stop {
        /// The terminal close reason.
        reason: CloseReason,
    },
}
