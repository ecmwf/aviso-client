// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Field-to-frame dispatcher per WHATWG §9.2.6.
//!
//! Maintains the four spec-defined buffers (`pending_event`,
//! `pending_data`, `last_event_id`, and an implicit reconnection time
//! which we surface as `Frame::Retry` rather than store) and converts
//! field updates plus blank-line dispatch triggers into `Frame`
//! values.

use core::mem;

use crate::frame::{Frame, Message, Retry};

#[derive(Debug, Default)]
pub(crate) struct Dispatcher {
    pending_event: String,
    pending_data: String,
    last_event_id: Option<String>,
}

impl Dispatcher {
    /// Process one field, returning a `Frame::Retry` if the field was
    /// a valid `retry:` directive. All other fields update internal
    /// state and return `None`.
    pub(crate) fn process_field(&mut self, name: &str, value: &str) -> Option<Frame> {
        match name {
            "event" => {
                value.clone_into(&mut self.pending_event);
                None
            }
            "data" => {
                self.pending_data.push_str(value);
                self.pending_data.push('\n');
                None
            }
            "id" => {
                if !value.contains('\0') {
                    self.last_event_id = Some(value.to_owned());
                }
                None
            }
            "retry" => Self::parse_retry(value).map(|millis| Frame::Retry(Retry { millis })),
            _ => None,
        }
    }

    /// Run the dispatch algorithm on a blank line. Returns a
    /// `Frame::Message` if the pending data buffer is non-empty;
    /// otherwise resets the per-event buffers and returns `None`.
    /// `last_event_id` is preserved either way.
    pub(crate) fn dispatch(&mut self) -> Option<Frame> {
        if self.pending_data.is_empty() {
            self.pending_event.clear();
            return None;
        }
        if self.pending_data.ends_with('\n') {
            self.pending_data.pop();
        }
        let message = Message {
            event: mem::take(&mut self.pending_event),
            data: mem::take(&mut self.pending_data),
            id: self.last_event_id.clone(),
        };
        Some(Frame::Message(message))
    }

    fn parse_retry(value: &str) -> Option<u64> {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        value.parse::<u64>().ok()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use super::Dispatcher;
    use crate::frame::{Frame, Message, Retry};

    fn msg(event: &str, data: &str, id: Option<&str>) -> Frame {
        Frame::Message(Message {
            event: event.to_owned(),
            data: data.to_owned(),
            id: id.map(str::to_owned),
        })
    }

    #[test]
    fn empty_dispatch_returns_none() {
        let mut d = Dispatcher::default();
        assert!(d.dispatch().is_none());
    }

    #[test]
    fn event_field_alone_does_not_dispatch() {
        let mut d = Dispatcher::default();
        assert!(d.process_field("event", "msg").is_none());
        assert!(d.dispatch().is_none(), "empty data suppresses dispatch");
    }

    #[test]
    fn data_field_dispatches_with_trailing_lf_stripped_once() {
        let mut d = Dispatcher::default();
        d.process_field("data", "hello");
        assert_eq!(d.dispatch(), Some(msg("", "hello", None)));
    }

    #[test]
    fn empty_data_value_dispatches_empty_message_not_suppressed() {
        // WPT: format-field-data.any.js. `data\n\n` (or `data:\n\n`)
        // appends `\n` to the buffer, so the buffer is "\n" (not empty)
        // at dispatch time. One LF is stripped, leaving an empty
        // message data string.
        let mut d = Dispatcher::default();
        d.process_field("data", "");
        assert_eq!(d.dispatch(), Some(msg("", "", None)));
    }

    #[test]
    fn multi_line_data_joined_with_lf() {
        let mut d = Dispatcher::default();
        d.process_field("data", "a");
        d.process_field("data", "b");
        assert_eq!(d.dispatch(), Some(msg("", "a\nb", None)));
    }

    #[test]
    fn data_after_data_empty_strips_only_one_trailing_lf() {
        // Per WHATWG: "remove the last character from the data buffer"
        // is exactly one character, not `trim_end_matches('\n')`.
        let mut d = Dispatcher::default();
        d.process_field("data", "a");
        d.process_field("data", "");
        assert_eq!(d.dispatch(), Some(msg("", "a\n", None)));
    }

    #[test]
    fn two_empty_data_lines_yield_single_lf_message() {
        // `data:\ndata:\n\n`: buffer goes "" -> "\n" -> "\n\n",
        // strip one LF -> "\n". Tests the strip-one-LF rule from a
        // second angle, with no leading non-empty content.
        let mut d = Dispatcher::default();
        d.process_field("data", "");
        d.process_field("data", "");
        assert_eq!(d.dispatch(), Some(msg("", "\n", None)));
    }

    #[test]
    fn event_resets_between_dispatches() {
        let mut d = Dispatcher::default();
        d.process_field("event", "first");
        d.process_field("data", "x");
        assert_eq!(d.dispatch(), Some(msg("first", "x", None)));
        d.process_field("data", "y");
        assert_eq!(
            d.dispatch(),
            Some(msg("", "y", None)),
            "event buffer must reset after dispatch"
        );
    }

    #[test]
    fn id_persists_across_dispatches() {
        let mut d = Dispatcher::default();
        d.process_field("id", "1");
        d.process_field("data", "x");
        assert_eq!(d.dispatch(), Some(msg("", "x", Some("1"))));
        d.process_field("data", "y");
        assert_eq!(d.dispatch(), Some(msg("", "y", Some("1"))));
    }

    #[test]
    fn id_with_nul_is_ignored_entirely() {
        // WPT: format-field-id-null.window.js
        let mut d = Dispatcher::default();
        d.process_field("id", "1");
        d.process_field("id", "bad\0value");
        d.process_field("data", "x");
        assert_eq!(
            d.dispatch(),
            Some(msg("", "x", Some("1"))),
            "NUL in id must leave the previous id intact"
        );
    }

    #[test]
    fn empty_id_clears_last_event_id_to_some_empty() {
        // WPT: format-field-id-3.window.js
        let mut d = Dispatcher::default();
        d.process_field("id", "1");
        d.process_field("data", "x");
        let _ = d.dispatch();
        d.process_field("id", "");
        d.process_field("data", "y");
        assert_eq!(d.dispatch(), Some(msg("", "y", Some(""))));
    }

    #[test]
    fn id_only_event_updates_last_id_but_emits_no_frame() {
        let mut d = Dispatcher::default();
        d.process_field("id", "42");
        assert!(d.dispatch().is_none(), "empty data suppresses dispatch");
        d.process_field("data", "next");
        assert_eq!(d.dispatch(), Some(msg("", "next", Some("42"))));
    }

    #[test]
    fn retry_with_digits_emits_retry_frame() {
        let mut d = Dispatcher::default();
        assert_eq!(
            d.process_field("retry", "1500"),
            Some(Frame::Retry(Retry { millis: 1500 }))
        );
    }

    #[test]
    fn retry_non_digit_is_ignored() {
        // WPT: format-field-retry-bogus.any.js
        let mut d = Dispatcher::default();
        assert!(d.process_field("retry", "foo").is_none());
        assert!(d.process_field("retry", "1 5 0 0").is_none());
        assert!(d.process_field("retry", "-100").is_none());
        assert!(d.process_field("retry", "1.5").is_none());
    }

    #[test]
    fn retry_empty_is_ignored() {
        let mut d = Dispatcher::default();
        assert!(d.process_field("retry", "").is_none());
    }

    #[test]
    fn retry_overflow_is_ignored() {
        let mut d = Dispatcher::default();
        assert!(
            d.process_field("retry", "99999999999999999999").is_none(),
            "u64 overflow must be ignored"
        );
    }

    #[test]
    fn unknown_field_has_no_effect() {
        let mut d = Dispatcher::default();
        assert!(d.process_field("custom", "value").is_none());
        d.process_field("data", "x");
        assert_eq!(d.dispatch(), Some(msg("", "x", None)));
    }
}
