// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The parser holds bytes until a terminator or a blank line arrives.
//! These tests feed it streams that never send one and check that it
//! stops at the configured bound, in time proportional to the bytes
//! fed, rather than growing without limit.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]

use std::time::{Duration, Instant};

use finesse::{Frame, Limits, Overflow, Parser};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// Feeds `total` bytes of `byte` in `chunk`-sized pieces, the way an HTTP
/// body arrives, and returns the first error.
fn feed_without_terminator(
    parser: &mut Parser,
    byte: u8,
    total: usize,
    chunk: usize,
) -> Option<Overflow> {
    let piece = vec![byte; chunk];
    let mut fed = 0;
    while fed < total {
        if let Err(e) = parser.feed(&piece) {
            return Some(e);
        }
        fed += chunk;
    }
    None
}

#[test]
fn a_line_that_never_ends_stops_at_the_bound() {
    let mut parser = Parser::new();
    let err = feed_without_terminator(&mut parser, b'x', 40 * MIB, 8 * KIB);
    assert_eq!(err, Some(Overflow::Line { max: 16 * MIB }));
    assert!(parser.next_frame().is_none());
}

#[test]
fn scanning_a_long_line_costs_time_proportional_to_its_length() {
    // With a bound far above the input, every chunk used to trigger a
    // rescan from the start of the buffer, so 32 MiB cost about a CPU
    // minute. Examining each byte once brings it to well under a second;
    // the budget below is loose enough for a slow CI machine and tight
    // enough that a quadratic scan cannot pass it.
    let limits = Limits::default().with_max_line_bytes(64 * MIB);
    let mut parser = Parser::with_limits(limits);
    let started = Instant::now();
    let err = feed_without_terminator(&mut parser, b'x', 32 * MIB, 8 * KIB);
    let elapsed = started.elapsed();
    assert_eq!(err, None);
    assert!(
        elapsed < Duration::from_secs(10),
        "32 MiB without a terminator took {elapsed:?}"
    );
}

#[test]
fn an_event_that_never_dispatches_stops_at_the_bound() {
    let limits = Limits::default().with_max_event_bytes(64 * KIB);
    let mut parser = Parser::with_limits(limits);
    let line = format!("data: {}\n", "y".repeat(KIB));
    let mut result = Ok(());
    for _ in 0..128 {
        result = parser.feed(line.as_bytes());
        if result.is_err() {
            break;
        }
    }
    assert_eq!(result, Err(Overflow::Event { max: 64 * KIB }));
}

#[test]
fn frames_complete_before_the_overflow_are_still_delivered() {
    let limits = Limits::default().with_max_line_bytes(KIB);
    let mut parser = Parser::with_limits(limits);
    parser.feed(b"data: first\n\n").unwrap();
    let err = feed_without_terminator(&mut parser, b'x', 4 * KIB, KIB);
    assert!(matches!(err, Some(Overflow::Line { .. })));
    match parser.next_frame() {
        Some(Frame::Message(message)) => assert_eq!(message.data, "first"),
        other => panic!("expected the completed message, got {other:?}"),
    }
    assert!(parser.next_frame().is_none());
    // The parser has ended: more bytes change nothing.
    parser.feed(b"data: late\n\n").unwrap();
    assert!(parser.next_frame().is_none());
}

#[test]
fn a_held_carriage_return_is_examined_again_when_the_next_byte_arrives() {
    // The splitter remembers where it stopped scanning. A CR at the end
    // of a chunk must not be skipped when the LF arrives in the next one.
    let mut parser = Parser::new();
    parser.feed(b"data: a\r").unwrap();
    parser.feed(b"\ndata: b\r\n\r\n").unwrap();
    parser.end().unwrap();
    match parser.next_frame() {
        Some(Frame::Message(message)) => assert_eq!(message.data, "a\nb"),
        other => panic!("expected one message, got {other:?}"),
    }
}

#[test]
fn the_defaults_are_generous() {
    let limits = Limits::default();
    assert_eq!(limits.max_line_bytes, 16 * MIB);
    assert_eq!(limits.max_event_bytes, 32 * MIB);
    assert_eq!(
        Overflow::Line { max: MIB }.to_string(),
        "SSE line exceeds 1048576 bytes"
    );
    // A notification the server's store can hold passes untouched.
    let mut parser = Parser::new();
    let big = format!("data: {}\n\n", "p".repeat(2 * MIB));
    parser.feed(big.as_bytes()).unwrap();
    assert!(matches!(parser.next_frame(), Some(Frame::Message(_))));
}

#[test]
fn one_chunk_larger_than_the_bound_is_not_held_whole() {
    // The bound is checked as the chunk is copied in, so the parser stops
    // soon after the bound rather than after the whole chunk.
    let limits = Limits::default().with_max_line_bytes(KIB);
    let mut parser = Parser::with_limits(limits);
    let chunk = vec![b'x'; 8 * MIB];
    let started = Instant::now();
    assert_eq!(parser.feed(&chunk), Err(Overflow::Line { max: KIB }));
    // Copying 8 MiB would take longer than this, and the error arrives
    // after the first 64 KiB piece either way.
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_completed_line_longer_than_the_bound_is_refused_too() {
    let limits = Limits::default().with_max_line_bytes(8);
    let mut parser = Parser::with_limits(limits);
    // Within the bound: fine, whatever the terminator.
    parser.feed(b"data: x\r\n\r\n").unwrap();
    assert!(matches!(parser.next_frame(), Some(Frame::Message(_))));
    // Nine bytes before the terminator, arriving in one piece.
    assert_eq!(
        parser.feed(b"data: yyy\n\n"),
        Err(Overflow::Line { max: 8 })
    );
}

#[test]
fn the_bound_counts_line_content_only_however_the_bytes_are_chunked() {
    // A line exactly at the bound, with its CR arriving at the end of one
    // chunk and the LF in the next, is within the bound: the held CR is a
    // terminator, not content.
    let limits = Limits::default().with_max_line_bytes(7);
    let mut parser = Parser::with_limits(limits);
    parser.feed(b"data: x\r").unwrap();
    parser.feed(b"\n\n").unwrap();
    assert!(matches!(parser.next_frame(), Some(Frame::Message(_))));

    // A BOM split across chunks is not content either. With a bound of
    // one byte, the one-chunk parse of the same bytes succeeds, and so
    // must the chunked one.
    let limits = Limits::default().with_max_line_bytes(1);
    let mut parser = Parser::with_limits(limits);
    parser.feed(b"\xEF").unwrap();
    parser.feed(b"\xBB").unwrap();
    parser.feed(b"\xBFx\n").unwrap();
    parser.end().unwrap();
    let mut whole = Parser::with_limits(limits);
    whole.feed(b"\xEF\xBB\xBFx\n").unwrap();
    whole.end().unwrap();
}

#[test]
fn an_overflow_completed_by_end_of_stream_is_reported() {
    // A trailing CR is held until the next byte or the end of the stream
    // decides what it is. At the end it completes the line, and that line
    // can take an event past its bound; the report must not be lost.
    // "xyz" plus the newline the dispatcher adds is four bytes.
    let limits = Limits::default().with_max_event_bytes(3);
    let mut parser = Parser::with_limits(limits);
    parser.feed(b"data: xyz\r").unwrap();
    assert_eq!(parser.end(), Err(Overflow::Event { max: 3 }));
    assert!(parser.next_frame().is_none());
}
