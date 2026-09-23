// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Incremental line splitter for the WHATWG §9.2.5 byte stream.
//!
//! Operates purely on bytes. Strips one leading UTF-8 BOM (with
//! correct cross-chunk lookahead) and emits one line at a time on
//! request. Recognises `\r\n`, lone `\n`, and lone `\r` as terminators
//! per spec, holding a trailing `\r` for one byte of lookahead so a
//! CR that turns out to be the first half of a CRLF does not falsely
//! emit an empty line.
//!
//! Each byte is examined once. A scan that finds no terminator
//! remembers where it stopped, so the next chunk does not make the
//! splitter re-read everything it has already seen. A line that grows
//! past the configured bound without a terminator is reported as an
//! [`Overflow`] instead of being kept.

use crate::limits::Overflow;

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

#[derive(Debug, Default, PartialEq, Eq)]
enum BomState {
    #[default]
    Pristine,
    Resolved,
}

#[derive(Debug)]
pub(crate) struct LineSplitter {
    buf: Vec<u8>,
    /// First byte of `buf` not yet examined for a terminator. Everything
    /// before it was scanned by an earlier `next_line` that found none.
    scan_from: usize,
    max_line_bytes: usize,
    bom_state: BomState,
    closed: bool,
}

impl Default for LineSplitter {
    fn default() -> Self {
        Self::new(crate::limits::Limits::default().max_line_bytes)
    }
}

impl LineSplitter {
    pub(crate) fn new(max_line_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            scan_from: 0,
            max_line_bytes,
            bom_state: BomState::default(),
            closed: false,
        }
    }

    pub(crate) fn feed(&mut self, chunk: &[u8]) {
        if self.closed {
            return;
        }
        self.buf.extend_from_slice(chunk);
    }

    pub(crate) fn end(&mut self) {
        self.closed = true;
    }

    /// Drains and returns the next completed line (without terminator
    /// bytes). Returns `Ok(None)` when more input is needed or when the
    /// stream is closed and exhausted, and `Err` when the bytes held
    /// without a terminator exceed the bound.
    pub(crate) fn next_line(&mut self) -> Result<Option<Vec<u8>>, Overflow> {
        self.resolve_bom();

        let mut i = self.scan_from;
        while i < self.buf.len() {
            match self.buf[i] {
                b'\n' => return self.take_line(i, i + 1),
                b'\r' => {
                    if i + 1 < self.buf.len() {
                        let consumed = if self.buf[i + 1] == b'\n' {
                            i + 2
                        } else {
                            i + 1
                        };
                        return self.take_line(i, consumed);
                    }
                    if self.closed {
                        return self.take_line(i, i + 1);
                    }
                    // Hold the CR: the next byte decides whether it is
                    // half of a CRLF. Look at it again next time.
                    self.scan_from = i;
                    return self.check_bound();
                }
                _ => {
                    i = i.saturating_add(1);
                }
            }
        }
        self.scan_from = self.buf.len();
        self.check_bound()
    }

    fn check_bound(&self) -> Result<Option<Vec<u8>>, Overflow> {
        if self.buf.len() > self.max_line_bytes {
            return Err(Overflow::Line {
                max: self.max_line_bytes,
            });
        }
        Ok(None)
    }

    /// Removes the completed line from the buffer and returns it, unless
    /// it is longer than the bound: a line that completes in the same
    /// piece that carried it past the bound is refused like one that
    /// never completes.
    fn take_line(
        &mut self,
        line_end: usize,
        drain_through: usize,
    ) -> Result<Option<Vec<u8>>, Overflow> {
        if line_end > self.max_line_bytes {
            return Err(Overflow::Line {
                max: self.max_line_bytes,
            });
        }
        let line = self.buf[..line_end].to_vec();
        self.buf.drain(..drain_through);
        self.scan_from = 0;
        Ok(Some(line))
    }

    fn resolve_bom(&mut self) {
        if self.bom_state == BomState::Resolved {
            return;
        }
        if self.buf.len() >= BOM.len() {
            if self.buf.starts_with(BOM) {
                self.buf.drain(..BOM.len());
                self.scan_from = 0;
            }
            self.bom_state = BomState::Resolved;
            return;
        }
        if !BOM.starts_with(&self.buf) {
            self.bom_state = BomState::Resolved;
            return;
        }
        if self.closed {
            self.bom_state = BomState::Resolved;
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on Vec ops and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use super::LineSplitter;

    fn collect_all(mut s: LineSplitter) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(line) = s.next_line().unwrap() {
            out.push(line);
        }
        out
    }

    #[test]
    fn empty_stream() {
        let mut s = LineSplitter::default();
        s.end();
        assert!(s.next_line().unwrap().is_none());
    }

    #[test]
    fn single_lf() {
        let mut s = LineSplitter::default();
        s.feed(b"\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![Vec::<u8>::new()]);
    }

    #[test]
    fn single_cr() {
        let mut s = LineSplitter::default();
        s.feed(b"\r");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![Vec::<u8>::new()]);
    }

    #[test]
    fn single_crlf() {
        let mut s = LineSplitter::default();
        s.feed(b"\r\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![Vec::<u8>::new()]);
    }

    #[test]
    fn simple_lf_terminated() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"abc".to_vec()]);
    }

    #[test]
    fn cr_held_pending_lookahead() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\r");
        assert!(
            s.next_line().unwrap().is_none(),
            "trailing CR with stream open must wait for lookahead"
        );
    }

    #[test]
    fn cr_resolved_as_lone_when_closed() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\r");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"abc".to_vec()]);
    }

    #[test]
    fn cr_then_lf_in_next_chunk_is_crlf() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\r");
        assert!(s.next_line().unwrap().is_none());
        s.feed(b"\nxyz\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"abc".to_vec(), b"xyz".to_vec()]);
    }

    #[test]
    fn cr_then_non_lf_in_next_chunk_is_lone_cr() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\r");
        assert!(s.next_line().unwrap().is_none());
        s.feed(b"def\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"abc".to_vec(), b"def".to_vec()]);
    }

    #[test]
    fn two_blank_lines() {
        let mut s = LineSplitter::default();
        s.feed(b"\r\n\r\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![Vec::<u8>::new(), Vec::<u8>::new()]);
    }

    #[test]
    fn bom_only_then_end() {
        let mut s = LineSplitter::default();
        s.feed(b"\xEF\xBB\xBF");
        s.end();
        let lines = collect_all(s);
        assert!(lines.is_empty(), "BOM only produces no lines");
    }

    #[test]
    fn bom_then_data() {
        let mut s = LineSplitter::default();
        s.feed(b"\xEF\xBB\xBFhello\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"hello".to_vec()]);
    }

    #[test]
    fn bom_split_across_chunks_1_2() {
        let mut s = LineSplitter::default();
        s.feed(b"\xEF");
        assert!(
            s.next_line().unwrap().is_none(),
            "must wait for the rest of the BOM"
        );
        s.feed(b"\xBB\xBFhello\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"hello".to_vec()]);
    }

    #[test]
    fn bom_split_across_chunks_2_1() {
        let mut s = LineSplitter::default();
        s.feed(b"\xEF\xBB");
        assert!(s.next_line().unwrap().is_none());
        s.feed(b"\xBFhello\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"hello".to_vec()]);
    }

    #[test]
    fn lone_ef_followed_by_terminator_is_data_not_bom() {
        // Confirms BOM detection does not eat a leading 0xEF byte when
        // the stream is too short to contain a full BOM. With a
        // terminator after the byte, the line is emitted as a single
        // 0xEF byte (which would later become a replacement character
        // when decoded as UTF-8 by the field layer).
        let mut s = LineSplitter::default();
        s.feed(b"\xEF\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"\xEF".to_vec()]);
    }

    #[test]
    fn lone_ef_without_terminator_is_discarded_at_eof() {
        // Per WHATWG: incomplete final line is discarded.
        let mut s = LineSplitter::default();
        s.feed(b"\xEF");
        s.end();
        let lines = collect_all(s);
        assert!(lines.is_empty());
    }

    #[test]
    fn ef_followed_by_non_bom_byte_is_data() {
        let mut s = LineSplitter::default();
        s.feed(b"\xEFabc\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"\xEFabc".to_vec()]);
    }

    #[test]
    fn mid_stream_bom_is_not_stripped() {
        let mut s = LineSplitter::default();
        s.feed(b"a\n\xEF\xBB\xBFb\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"a".to_vec(), b"\xEF\xBB\xBFb".to_vec()]);
    }

    #[test]
    fn feed_after_end_is_no_op() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\n");
        s.end();
        s.feed(b"def\n");
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"abc".to_vec()]);
    }

    #[test]
    fn line_with_no_terminator_is_held_until_close() {
        let mut s = LineSplitter::default();
        s.feed(b"abc");
        assert!(s.next_line().unwrap().is_none(), "must wait for terminator");
        s.end();
        assert!(
            s.next_line().unwrap().is_none(),
            "spec: incomplete final line is discarded"
        );
    }
}
