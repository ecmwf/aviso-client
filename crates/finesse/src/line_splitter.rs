//! Incremental line splitter for the WHATWG §9.2.5 byte stream.
//!
//! Operates purely on bytes. Strips one leading UTF-8 BOM (with
//! correct cross-chunk lookahead) and emits one line at a time on
//! request. Recognises `\r\n`, lone `\n`, and lone `\r` as terminators
//! per spec, holding a trailing `\r` for one byte of lookahead so a
//! CR that turns out to be the first half of a CRLF does not falsely
//! emit an empty line.

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

#[derive(Debug, Default, PartialEq, Eq)]
enum BomState {
    #[default]
    Pristine,
    Resolved,
}

#[derive(Debug, Default)]
pub(crate) struct LineSplitter {
    buf: Vec<u8>,
    bom_state: BomState,
    closed: bool,
}

impl LineSplitter {
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
    /// bytes). Returns `None` when more input is needed or when the
    /// stream is closed and exhausted.
    pub(crate) fn next_line(&mut self) -> Option<Vec<u8>> {
        self.resolve_bom();

        let mut i = 0;
        while i < self.buf.len() {
            match self.buf[i] {
                b'\n' => return Some(self.take_line(i, i + 1)),
                b'\r' => {
                    if i + 1 < self.buf.len() {
                        let consumed = if self.buf[i + 1] == b'\n' {
                            i + 2
                        } else {
                            i + 1
                        };
                        return Some(self.take_line(i, consumed));
                    }
                    if self.closed {
                        return Some(self.take_line(i, i + 1));
                    }
                    return None;
                }
                _ => {
                    i = i.saturating_add(1);
                }
            }
        }
        None
    }

    fn take_line(&mut self, line_end: usize, drain_through: usize) -> Vec<u8> {
        let line = self.buf[..line_end].to_vec();
        self.buf.drain(..drain_through);
        line
    }

    fn resolve_bom(&mut self) {
        if self.bom_state == BomState::Resolved {
            return;
        }
        if self.buf.len() >= BOM.len() {
            if self.buf.starts_with(BOM) {
                self.buf.drain(..BOM.len());
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
        while let Some(line) = s.next_line() {
            out.push(line);
        }
        out
    }

    #[test]
    fn empty_stream() {
        let mut s = LineSplitter::default();
        s.end();
        assert!(s.next_line().is_none());
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
            s.next_line().is_none(),
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
        assert!(s.next_line().is_none());
        s.feed(b"\nxyz\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"abc".to_vec(), b"xyz".to_vec()]);
    }

    #[test]
    fn cr_then_non_lf_in_next_chunk_is_lone_cr() {
        let mut s = LineSplitter::default();
        s.feed(b"abc\r");
        assert!(s.next_line().is_none());
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
        assert!(s.next_line().is_none(), "must wait for the rest of the BOM");
        s.feed(b"\xBB\xBFhello\n");
        s.end();
        let lines = collect_all(s);
        assert_eq!(lines, vec![b"hello".to_vec()]);
    }

    #[test]
    fn bom_split_across_chunks_2_1() {
        let mut s = LineSplitter::default();
        s.feed(b"\xEF\xBB");
        assert!(s.next_line().is_none());
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
        assert!(s.next_line().is_none(), "must wait for terminator");
        s.end();
        assert!(
            s.next_line().is_none(),
            "spec: incomplete final line is discarded"
        );
    }
}
