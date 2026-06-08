// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Field-line parsing per WHATWG §9.2.6.
//!
//! Takes a line (the byte sequence the splitter produced, with the
//! terminator already removed) and classifies it as one of:
//! `Blank` (the dispatch trigger), `Comment` (line starting with `:`,
//! ignored), or `Field { name, value }`. Field names and values are
//! decoded from bytes via the UTF-8 decode algorithm with
//! replacement, matching the spec's "decoded using the UTF-8 decode
//! algorithm" requirement.

/// What a single decoded line represents.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// Empty line; triggers event dispatch.
    Blank,
    /// Line beginning with `:`; ignored.
    Comment,
    /// Field with a parsed name and value.
    Field {
        /// UTF-8 decoded field name with replacement on invalid bytes.
        name: String,
        /// UTF-8 decoded field value with replacement on invalid bytes.
        value: String,
    },
}

/// Classify a single line (without its terminator) per WHATWG §9.2.6.
pub(crate) fn classify_line(line: &[u8]) -> LineKind {
    if line.is_empty() {
        return LineKind::Blank;
    }
    if line[0] == b':' {
        return LineKind::Comment;
    }
    let (name_bytes, value_bytes) = match line.iter().position(|&b| b == b':') {
        Some(colon_pos) => {
            let value_start = colon_pos.saturating_add(1);
            let mut value = &line[value_start..];
            if value.first() == Some(&b' ') {
                value = &value[1..];
            }
            (&line[..colon_pos], value)
        }
        None => (line, &[][..]),
    };
    LineKind::Field {
        name: String::from_utf8_lossy(name_bytes).into_owned(),
        value: String::from_utf8_lossy(value_bytes).into_owned(),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use super::{LineKind, classify_line};

    fn field(name: &str, value: &str) -> LineKind {
        LineKind::Field {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn empty_line_is_blank() {
        assert_eq!(classify_line(b""), LineKind::Blank);
    }

    #[test]
    fn leading_colon_is_comment() {
        assert_eq!(classify_line(b":"), LineKind::Comment);
        assert_eq!(classify_line(b":keepalive"), LineKind::Comment);
        assert_eq!(classify_line(b": with text"), LineKind::Comment);
    }

    #[test]
    fn field_with_value_and_single_space_stripped() {
        assert_eq!(classify_line(b"data: hello"), field("data", "hello"));
    }

    #[test]
    fn field_without_leading_space() {
        assert_eq!(classify_line(b"data:hello"), field("data", "hello"));
    }

    #[test]
    fn field_strips_only_one_leading_space() {
        assert_eq!(classify_line(b"data:  hello"), field("data", " hello"));
    }

    #[test]
    fn field_with_no_colon_is_name_only() {
        assert_eq!(classify_line(b"data"), field("data", ""));
    }

    #[test]
    fn field_with_colon_and_empty_value() {
        assert_eq!(classify_line(b"data:"), field("data", ""));
    }

    #[test]
    fn field_with_colon_and_single_space_value_strips_to_empty() {
        assert_eq!(classify_line(b"data: "), field("data", ""));
    }

    #[test]
    fn invalid_utf8_in_value_becomes_replacement_character() {
        let LineKind::Field { name, value } = classify_line(b"data: \xFF\xFE") else {
            panic!("expected field");
        };
        assert_eq!(name, "data");
        assert!(value.chars().all(|c| c == '\u{FFFD}'));
    }

    #[test]
    fn invalid_utf8_in_name_becomes_replacement_character() {
        let LineKind::Field { name, value } = classify_line(b"\xFFname: value") else {
            panic!("expected field");
        };
        assert!(name.starts_with('\u{FFFD}'));
        assert_eq!(value, "value");
    }

    #[test]
    fn colon_at_end_is_name_with_empty_value() {
        assert_eq!(classify_line(b"abc:"), field("abc", ""));
    }
}
