// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Neutralises notification values for a webhook URL.
//!
//! [`percent_encode`] keeps RFC 3986 unreserved characters and encodes
//! every other byte, so a value cannot add path segments or query
//! parameters. Encoding cannot stop a value from being the host when the
//! operator put the placeholder there, so [`UrlTracker`] follows the
//! rendered text and reports whether the path has started; the engine
//! refuses a notification value before that point.

/// Which part of a URL the rendered text has reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UrlTracker {
    /// How many bytes of `://` have matched so far, across pieces.
    separator_matched: u8,
    seen_scheme_separator: bool,
    in_path: bool,
}

impl UrlTracker {
    pub(super) fn new() -> Self {
        Self {
            separator_matched: 0,
            seen_scheme_separator: false,
            in_path: false,
        }
    }

    /// Reads a piece of the rendered URL. The authority runs from `://`
    /// to the first `/`, `?` or `#`; everything after that is path,
    /// query or fragment.
    ///
    /// Valid: `https://h` ends in the authority; `https://h/` and
    /// `https://h?q` have reached the path.
    pub(super) fn advance(&mut self, text: &str) {
        for byte in text.bytes() {
            if self.in_path {
                return;
            }
            if self.seen_scheme_separator {
                self.in_path = matches!(byte, b'/' | b'?' | b'#');
                continue;
            }
            // Match `://` one byte at a time so the three bytes may arrive
            // in different pieces.
            self.separator_matched = match (self.separator_matched, byte) {
                (_, b':') => 1,
                (1 | 2, b'/') => self.separator_matched + 1,
                _ => 0,
            };
            if self.separator_matched == 3 {
                self.seen_scheme_separator = true;
            }
        }
    }

    /// True once the scheme and authority are behind us.
    pub(super) fn in_path(self) -> bool {
        self.in_path
    }
}

/// Percent-encodes every byte of `value` except the RFC 3986 unreserved
/// set (`A-Z a-z 0-9 - . _ ~`).
///
/// Valid: `mars` stays `mars`; `a/b?c=1` becomes `a%2Fb%3Fc%3D1`.
pub(super) fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_tracker_knows_when_the_path_starts() {
        let reached = |text: &str| {
            let mut tracker = UrlTracker::new();
            tracker.advance(text);
            tracker.in_path()
        };
        assert!(!reached(""));
        assert!(!reached("https://"));
        assert!(!reached("https://hooks.example"));
        assert!(!reached("https://user:pw@hooks.example:8443"));
        assert!(reached("https://hooks.example/"));
        assert!(reached("https://hooks.example?q=1"));
        assert!(reached("https://hooks.example#f"));
        // Pieces arrive one at a time, an env value among them.
        let mut tracker = UrlTracker::new();
        tracker.advance("https://hooks.example");
        assert!(!tracker.in_path());
        tracker.advance("/notify/");
        assert!(tracker.in_path());
        // The separator itself may be split across pieces.
        let mut tracker = UrlTracker::new();
        tracker.advance("https:");
        tracker.advance("/");
        tracker.advance("/");
        assert!(!tracker.in_path());
        tracker.advance("evil.example");
        assert!(!tracker.in_path());
        tracker.advance("/hook");
        assert!(tracker.in_path());
    }

    #[test]
    fn percent_encode_keeps_unreserved_and_encodes_the_rest() {
        assert_eq!(percent_encode("mars-2.0_x~"), "mars-2.0_x~");
        assert_eq!(percent_encode("a/b?c=1&d"), "a%2Fb%3Fc%3D1%26d");
        assert_eq!(percent_encode("{\"a\":1}"), "%7B%22a%22%3A1%7D");
        assert_eq!(percent_encode("é"), "%C3%A9");
    }
}
