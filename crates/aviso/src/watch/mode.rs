// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Watch session mode: historical-then-live versus replay-only.

/// Mode of a watch session.
///
/// The mode is fixed for the lifetime of a session and feeds into the
/// reducer's handling of `end_of_stream` and `replay_completed` events
/// (D2: replay-only terminates on `end_of_stream` after
/// `replay_completed`; watch reconnects on `end_of_stream` at any
/// time).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchMode {
    /// Historical-then-live: replay any backlog, then stay connected
    /// for live notifications. Reconnects on `end_of_stream`.
    Watch,

    /// Replay-only: replay the requested range and stop. Terminates on
    /// `end_of_stream` once the server's `replay_completed` event has
    /// been received.
    ReplayOnly,
}
