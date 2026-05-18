//! Committed position in a watch stream.

use serde::{Deserialize, Serialize};

/// A committed position in a watch stream, keyed by [`ResumeKey`](super::ResumeKey).
///
/// `last_committed_sequence` is canonical: the supervisor reconnects
/// with `from_id = last_committed_sequence + 1`. The reconnect path
/// does not consult `last_event_id`; that field exists purely for
/// human-readable logging and operator debugging.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Canonical resume cursor. The numeric sequence from the
    /// `CloudEvent` `id` of the last successfully processed notification.
    pub last_committed_sequence: u64,
    /// Full `CloudEvent` id (`<event_type>@<sequence>`) of the last
    /// committed notification. Diagnostic only; not used by the
    /// reconnect path.
    pub last_event_id: Option<String>,
}

impl Checkpoint {
    /// Construct a checkpoint at the given sequence with an optional
    /// diagnostic event id.
    #[must_use]
    pub fn new(last_committed_sequence: u64, last_event_id: Option<String>) -> Self {
        Self {
            last_committed_sequence,
            last_event_id,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use super::Checkpoint;

    #[test]
    fn serde_roundtrip_with_id() {
        let cp = Checkpoint::new(42, Some("mars@42".into()));
        let json = serde_json::to_string(&cp).unwrap();
        let back: Checkpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(cp, back);
    }

    #[test]
    fn serde_roundtrip_without_id() {
        let cp = Checkpoint::new(7, None);
        let json = serde_json::to_string(&cp).unwrap();
        let back: Checkpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(cp, back);
    }

    #[test]
    fn new_sets_fields() {
        let cp = Checkpoint::new(1, Some("e@1".into()));
        assert_eq!(cp.last_committed_sequence, 1);
        assert_eq!(cp.last_event_id.as_deref(), Some("e@1"));
    }
}
