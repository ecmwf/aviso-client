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
    let err = feed_without_terminator(&mut parser, b'x', 4 * MIB, 8 * KIB);
    assert_eq!(err, Some(Overflow::Line { max: MIB }));
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
    parser.end();
    match parser.next_frame() {
        Some(Frame::Message(message)) => assert_eq!(message.data, "a\nb"),
        other => panic!("expected one message, got {other:?}"),
    }
}

#[test]
fn the_defaults_are_generous() {
    let limits = Limits::default();
    assert_eq!(limits.max_line_bytes, MIB);
    assert_eq!(limits.max_event_bytes, 8 * MIB);
    assert_eq!(
        Overflow::Line { max: MIB }.to_string(),
        "SSE line exceeds 1048576 bytes without a terminator"
    );
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
