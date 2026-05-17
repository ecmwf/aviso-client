//! Public frame types emitted by [`Parser`](crate::Parser).
//!
//! The two variants mirror the WHATWG §9.2.6 dispatch outcomes: a
//! `Message` is produced when a blank line follows a non-empty data
//! buffer, and a `Retry` is produced as soon as a valid `retry:`
//! directive is parsed. Every public type carries `#[non_exhaustive]`
//! so future spec or aviso extensions can add fields or variants
//! without breaking exhaustive matches in downstream code.

/// A dispatched SSE event with its final accumulated buffers.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    /// Event type buffer at dispatch. Empty when no `event:` field
    /// appeared in this event.
    pub event: String,
    /// Data buffer at dispatch with the single trailing LF removed
    /// per spec.
    pub data: String,
    /// Last event ID buffer at dispatch. `None` until the first `id:`
    /// field is parsed; `Some("")` if an `id:` field with an empty
    /// value explicitly cleared it.
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
