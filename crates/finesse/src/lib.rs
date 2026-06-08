// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! WHATWG Server-Sent Events parser. Sync, push-based, no I/O.
//!
//! `finesse` implements the parsing algorithm from the HTML Living
//! Standard, sections 9.2.5 (parsing an event stream) and 9.2.6
//! (interpreting an event stream). It takes raw bytes from any
//! source and yields typed [`Frame`] values; it owns no HTTP
//! transport, no reconnect logic, and no aviso-specific semantics.
//! Those concerns belong to the consumer.
//!
//! ```
//! use finesse::{Frame, Parser};
//!
//! let mut parser = Parser::new();
//! parser.feed(b"event: ping\ndata: hello\n\n");
//! parser.end();
//!
//! match parser.next_frame() {
//!     Some(Frame::Message(msg)) => {
//!         assert_eq!(msg.event, "ping");
//!         assert_eq!(msg.data, "hello");
//!         assert_eq!(msg.id, None);
//!     }
//!     other => unreachable!("expected Message, got {other:?}"),
//! }
//! assert!(parser.next_frame().is_none());
//! ```
//!
//! finesse owns no transport, no async runtime, and no aviso
//! semantics. It is a state machine the caller drives with `feed`
//! and drains with `next_frame`.

#![forbid(unsafe_code)]

mod dispatcher;
mod field;
mod frame;
mod line_splitter;

use std::collections::VecDeque;

use crate::dispatcher::Dispatcher;
use crate::field::{LineKind, classify_line};
use crate::line_splitter::LineSplitter;

pub use crate::frame::{Frame, Message, Retry};

/// A WHATWG Server-Sent Events parser.
///
/// Feed it bytes via [`feed`](Self::feed), drain ready frames via
/// [`next_frame`](Self::next_frame), and signal end-of-stream via
/// [`end`](Self::end). The parser owns its byte buffer; emitted
/// frames own their `String` payloads.
#[derive(Debug, Default)]
pub struct Parser {
    line_splitter: LineSplitter,
    dispatcher: Dispatcher,
    queue: VecDeque<Frame>,
    closed: bool,
}

impl Parser {
    /// Create a new parser ready to consume bytes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes from the wire.
    ///
    /// The internal byte buffer grows as needed. After [`end`](Self::end)
    /// has been called, further `feed` calls are silently ignored.
    pub fn feed(&mut self, chunk: &[u8]) {
        if self.closed {
            return;
        }
        self.line_splitter.feed(chunk);
        self.drain_lines();
    }

    /// Signal end-of-stream.
    ///
    /// Resolves any held line-terminator state: a trailing CR that
    /// was awaiting a possible LF lookahead becomes a lone-CR
    /// terminator. Per WHATWG §9.2.6, any pending event without a
    /// trailing blank line is discarded; this method does NOT
    /// dispatch it. Idempotent.
    pub fn end(&mut self) {
        if self.closed {
            return;
        }
        self.line_splitter.end();
        self.drain_lines();
        self.closed = true;
    }

    /// Drain one ready [`Frame`].
    ///
    /// Returns `None` when the parser needs more bytes to complete
    /// the next frame, or when the stream has ended and the queue is
    /// empty.
    pub fn next_frame(&mut self) -> Option<Frame> {
        self.queue.pop_front()
    }

    fn drain_lines(&mut self) {
        while let Some(line) = self.line_splitter.next_line() {
            match classify_line(&line) {
                LineKind::Blank => {
                    if let Some(frame) = self.dispatcher.dispatch() {
                        self.queue.push_back(frame);
                    }
                }
                LineKind::Comment => {}
                LineKind::Field { name, value } => {
                    if let Some(frame) = self.dispatcher.process_field(&name, &value) {
                        self.queue.push_back(frame);
                    }
                }
            }
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
    use super::{Frame, Message, Parser, Retry};

    fn collect(mut p: Parser) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(frame) = p.next_frame() {
            out.push(frame);
        }
        out
    }

    fn message(event: &str, data: &str, id: Option<&str>) -> Frame {
        Frame::Message(Message {
            event: event.to_owned(),
            data: data.to_owned(),
            id: id.map(str::to_owned),
        })
    }

    #[test]
    fn empty_stream_yields_no_frames() {
        let mut p = Parser::new();
        p.end();
        assert!(collect(p).is_empty());
    }

    #[test]
    fn single_message_event() {
        let mut p = Parser::new();
        p.feed(b"data: hello\n\n");
        p.end();
        assert_eq!(collect(p), vec![message("", "hello", None)]);
    }

    #[test]
    fn event_field_carries_through() {
        let mut p = Parser::new();
        p.feed(b"event: ping\ndata: hello\n\n");
        p.end();
        assert_eq!(collect(p), vec![message("ping", "hello", None)]);
    }

    #[test]
    fn two_consecutive_events_in_one_chunk() {
        let mut p = Parser::new();
        p.feed(b"data: one\n\ndata: two\n\n");
        p.end();
        assert_eq!(
            collect(p),
            vec![message("", "one", None), message("", "two", None)]
        );
    }

    #[test]
    fn retry_emits_retry_frame_before_message() {
        let mut p = Parser::new();
        p.feed(b"retry: 1500\ndata: x\n\n");
        p.end();
        assert_eq!(
            collect(p),
            vec![Frame::Retry(Retry { millis: 1500 }), message("", "x", None)]
        );
    }

    #[test]
    fn chunked_feed_works_at_any_boundary() {
        let mut p = Parser::new();
        p.feed(b"da");
        p.feed(b"ta: he");
        p.feed(b"llo\n");
        p.feed(b"\n");
        p.end();
        assert_eq!(collect(p), vec![message("", "hello", None)]);
    }

    #[test]
    fn id_persists_across_events() {
        let mut p = Parser::new();
        p.feed(b"id: 1\ndata: a\n\ndata: b\n\n");
        p.end();
        assert_eq!(
            collect(p),
            vec![message("", "a", Some("1")), message("", "b", Some("1"))]
        );
    }

    #[test]
    fn end_with_held_cr_does_not_dispatch_incomplete_event() {
        let mut p = Parser::new();
        p.feed(b"data: x\r");
        p.end();
        assert!(collect(p).is_empty(), "no blank line means no dispatch");
    }

    #[test]
    fn end_is_idempotent() {
        let mut p = Parser::new();
        p.feed(b"data: x\n\n");
        p.end();
        p.end();
        p.end();
        assert_eq!(collect(p), vec![message("", "x", None)]);
    }

    #[test]
    fn feed_after_end_is_no_op() {
        let mut p = Parser::new();
        p.feed(b"data: x\n\n");
        p.end();
        p.feed(b"data: y\n\n");
        assert_eq!(
            collect(p),
            vec![message("", "x", None)],
            "second event must not be parsed after end()"
        );
    }

    #[test]
    fn bom_stripped_at_start() {
        let mut p = Parser::new();
        p.feed(b"\xEF\xBB\xBFdata: hello\n\n");
        p.end();
        assert_eq!(collect(p), vec![message("", "hello", None)]);
    }

    #[test]
    fn comment_line_ignored() {
        let mut p = Parser::new();
        p.feed(b":keepalive\ndata: hello\n\n");
        p.end();
        assert_eq!(collect(p), vec![message("", "hello", None)]);
    }

    #[test]
    fn crlf_terminators() {
        let mut p = Parser::new();
        p.feed(b"data: a\r\ndata: b\r\n\r\n");
        p.end();
        assert_eq!(collect(p), vec![message("", "a\nb", None)]);
    }

    #[test]
    fn event_only_block_emits_no_frame() {
        // WHATWG: empty data buffer at dispatch suppresses the
        // message and resets event-type and data buffers. The
        // event-type buffer must not leak into the next event.
        let mut p = Parser::new();
        p.feed(b"event: heartbeat\n\ndata: real\n\n");
        p.end();
        assert_eq!(
            collect(p),
            vec![message("", "real", None)],
            "heartbeat-only block must not produce a Message AND must not leak its event type"
        );
    }

    #[test]
    fn id_only_block_updates_last_id_without_emitting() {
        let mut p = Parser::new();
        p.feed(b"id: 42\n\ndata: next\n\n");
        p.end();
        assert_eq!(collect(p), vec![message("", "next", Some("42"))]);
    }

    #[test]
    fn invalid_utf8_in_data_becomes_replacement_chars() {
        let mut p = Parser::new();
        p.feed(b"data: \xFF\xFE\n\n");
        p.end();
        let frames = collect(p);
        assert_eq!(frames.len(), 1);
        let Frame::Message(Message { data, .. }) = &frames[0] else {
            panic!("expected message");
        };
        assert!(
            data.chars().all(|c| c == '\u{FFFD}'),
            "invalid UTF-8 must surface as replacement characters"
        );
    }
}
