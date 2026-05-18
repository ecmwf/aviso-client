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
