// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Public frame types emitted by [`Parser`](crate::Parser).
//!
//! The two variants mirror the WHATWG §9.2.6 dispatch outcomes: a
//! `Message` is produced when a blank line follows a non-empty data
//! buffer, and a `Retry` is produced as soon as a valid `retry:`
//! directive is parsed. Every public type carries `#[non_exhaustive]`
//! so future spec or aviso extensions can add fields or variants
//! without breaking exhaustive matches in downstream code.

/// A dispatched SSE event with its final accumulated buffers.
///
/// These fields mirror the WHATWG parser's internal buffers, not the
/// DOM-level [`MessageEvent`] interface that browsers ultimately
/// expose. In particular: `event` stays empty (not defaulted to
/// `"message"`) when no `event:` field appeared, and `id` stays
/// `None` (not `""`) before the first `id:` field is parsed. The
/// browser's defaulting is a layer-two concern and lives in the
/// consumer that maps `finesse::Frame` to its own typed event model.
///
/// [`MessageEvent`]: https://html.spec.whatwg.org/multipage/comms.html#messageevent
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    /// Event type buffer at dispatch. Empty when no `event:` field
    /// appeared in this event. The browser-level default of
    /// `"message"` is deliberately NOT applied here; see the type
    /// doc comment.
    pub event: String,
    /// Data buffer at dispatch with the single trailing LF removed
    /// per spec.
    pub data: String,
    /// Last event ID buffer at dispatch. `None` until the first `id:`
    /// field is parsed; `Some("")` if an `id:` field with an empty
    /// value explicitly cleared it. The spec's initial empty-string
    /// default is deliberately NOT applied here; see the type doc
    /// comment.
    pub id: Option<String>,
}

/// A reconnection-time directive parsed from a `retry:` line.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// Reconnection time in milliseconds. The WHATWG spec defines the
    /// integer's units only via "the event stream's reconnection
    /// time"; servers use milliseconds by universal convention.
    pub millis: u64,
}

/// A frame emitted by [`Parser`](crate::Parser).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A dispatched event.
    Message(Message),
    /// A reconnection-time directive.
    Retry(Retry),
}
