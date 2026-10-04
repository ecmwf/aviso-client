// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The end point of a replay across reconnects.
//!
//! The server reports the end it resolved in `replay_started`. The first
//! value is kept and sent as `to_id` on every later connection, so a date
//! end cannot move. Once the notification at a sequence end has been
//! delivered, the replay is finished even if the connection dropped before
//! the server confirmed it: reconnecting would at best deliver it again,
//! and before the first commit, when the cursor is the pending notification,
//! it would send a start past the end, which the server refuses.

use super::PendingCommit;
use crate::watch::ReplayEnd;

/// The end point to send: the end the server resolved, else the requested
/// one.
pub(super) fn effective(resolved: Option<u64>, requested: Option<&ReplayEnd>) -> Option<ReplayEnd> {
    resolved
        .map(ReplayEnd::Sequence)
        .or_else(|| requested.cloned())
}

/// Whether everything up to a sequence end has been delivered. The pending
/// notification is the last one delivered; without one, the committed
/// cursor is. A date end is never known to be reached until the server
/// resolves it to a sequence.
pub(super) fn reached(
    end: Option<&ReplayEnd>,
    pending: Option<&PendingCommit>,
    committed: Option<u64>,
) -> bool {
    let Some(ReplayEnd::Sequence(end)) = end else {
        return false;
    };
    super::last_delivered(pending, committed).is_some_and(|delivered| delivered >= *end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(sequence: u64) -> PendingCommit {
        PendingCommit {
            sequence,
            event_id: format!("mars@{sequence}"),
        }
    }

    #[test]
    fn the_resolved_end_wins_over_the_requested_one() {
        let date = ReplayEnd::Date("2026-09-02T00:00:00Z".into());
        assert_eq!(
            effective(Some(7), Some(&date)),
            Some(ReplayEnd::Sequence(7))
        );
        assert_eq!(effective(None, Some(&date)), Some(date));
        assert_eq!(effective(None, None), None);
    }

    #[test]
    fn a_sequence_end_is_reached_once_its_notification_is_delivered() {
        let end = ReplayEnd::Sequence(5);
        assert!(!reached(Some(&end), None, None));
        assert!(!reached(Some(&end), Some(&pending(4)), Some(3)));
        assert!(reached(Some(&end), Some(&pending(5)), None));
        assert!(reached(Some(&end), None, Some(6)));
    }

    #[test]
    fn a_date_end_or_no_end_is_never_reached_here() {
        let date = ReplayEnd::Date("2026-09-02T00:00:00Z".into());
        assert!(!reached(Some(&date), Some(&pending(100)), None));
        assert!(!reached(None, Some(&pending(100)), None));
    }
}
