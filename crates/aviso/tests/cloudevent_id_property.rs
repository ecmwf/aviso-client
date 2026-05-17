//! Property tests for the `CloudEvent` id parser.
//!
//! Exercises the round-trip and rejection invariants of [`aviso::parse_cloudevent_id`] over
//! generated input rather than hand-picked cases.

#![allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on a parser success is the expected diagnostic"
)]

use aviso::parse_cloudevent_id;
use proptest::prelude::*;

proptest! {
    /// Any valid `<event_type>@<sequence>` constructed by the test must parse back to the same
    /// parts. The event-type regex excludes `@` so the rsplit always isolates the sequence.
    #[test]
    fn round_trips_valid_ids(
        event_type in "[a-zA-Z0-9_.\\-]+",
        sequence in 0u64..=u64::MAX,
    ) {
        let id = format!("{event_type}@{sequence}");
        let (parsed_event_type, parsed_sequence) = parse_cloudevent_id(&id).unwrap();
        prop_assert_eq!(parsed_event_type, event_type);
        prop_assert_eq!(parsed_sequence, sequence);
    }

    /// Any string containing no `@` separator must be rejected.
    #[test]
    fn rejects_strings_without_separator(s in "[^@]+") {
        prop_assert!(parse_cloudevent_id(&s).is_err());
    }

    /// Any string whose suffix after the last `@` is not a u64 must be rejected.
    #[test]
    fn rejects_non_numeric_sequence_suffix(
        prefix in "[a-zA-Z0-9_.\\-]+",
        bad_suffix in "[a-zA-Z]+",
    ) {
        let id = format!("{prefix}@{bad_suffix}");
        prop_assert!(parse_cloudevent_id(&id).is_err());
    }
}
