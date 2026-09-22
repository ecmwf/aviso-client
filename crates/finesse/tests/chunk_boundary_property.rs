// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Chunk-boundary invariance for [`finesse::Parser`].
//!
//! Property: for any byte sequence `S` and any partition of `S` into
//! consecutive chunks `c1..cn`, the sequence of frames produced by
//! `feed(c1); ...; feed(cn); end()` equals the sequence produced by
//! `feed(S); end()`. This is the central correctness invariant for an
//! incremental parser; if it fails, every consumer that drives the
//! parser from a chunked HTTP body has a latent bug.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]

use finesse::{Frame, Parser};
use proptest::collection::vec;
use proptest::prelude::*;

fn parse_one_shot(bytes: &[u8]) -> Vec<Frame> {
    let mut parser = Parser::new();
    parser.feed(bytes).unwrap();
    parser.end();
    drain(&mut parser)
}

fn parse_in_chunks(bytes: &[u8], boundaries: &[usize]) -> Vec<Frame> {
    let mut boundaries: Vec<usize> = boundaries
        .iter()
        .copied()
        .filter(|&b| b <= bytes.len())
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut parser = Parser::new();
    let mut last = 0;
    for &b in &boundaries {
        parser.feed(&bytes[last..b]).unwrap();
        last = b;
    }
    parser.feed(&bytes[last..]).unwrap();
    parser.end();
    drain(&mut parser)
}

fn drain(parser: &mut Parser) -> Vec<Frame> {
    let mut out = Vec::new();
    while let Some(frame) = parser.next_frame() {
        out.push(frame);
    }
    out
}

fn well_formed_event() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::option::of("[a-zA-Z][a-zA-Z0-9_-]{0,15}"),
        vec("[^\r\n]{0,32}", 0..4),
        prop::option::of("[a-zA-Z0-9_-]{0,16}"),
        prop::option::of(0_u32..1_000_000),
        any::<bool>(),
    )
        .prop_map(|(event, data_lines, id, retry, crlf)| {
            let nl: &[u8] = if crlf { b"\r\n" } else { b"\n" };
            let mut out = Vec::new();
            if let Some(name) = event {
                out.extend_from_slice(b"event: ");
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(nl);
            }
            for line in &data_lines {
                out.extend_from_slice(b"data: ");
                out.extend_from_slice(line.as_bytes());
                out.extend_from_slice(nl);
            }
            if let Some(value) = id {
                out.extend_from_slice(b"id: ");
                out.extend_from_slice(value.as_bytes());
                out.extend_from_slice(nl);
            }
            if let Some(value) = retry {
                out.extend_from_slice(format!("retry: {value}").as_bytes());
                out.extend_from_slice(nl);
            }
            out.extend_from_slice(nl);
            out
        })
}

fn well_formed_stream() -> impl Strategy<Value = Vec<u8>> {
    vec(well_formed_event(), 0..6).prop_map(|events| events.into_iter().flatten().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn chunk_partition_invariant_well_formed(stream in well_formed_stream(), boundaries in vec(any::<usize>(), 0..16)) {
        let single = parse_one_shot(&stream);
        let chunked = parse_in_chunks(&stream, &boundaries);
        prop_assert_eq!(single, chunked);
    }

    #[test]
    fn chunk_partition_invariant_arbitrary_bytes(stream in vec(any::<u8>(), 0..256), boundaries in vec(any::<usize>(), 0..16)) {
        let single = parse_one_shot(&stream);
        let chunked = parse_in_chunks(&stream, &boundaries);
        prop_assert_eq!(single, chunked);
    }
}
