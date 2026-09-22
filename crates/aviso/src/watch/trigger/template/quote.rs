// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Neutralises substituted notification values for the text they land in.
//!
//! A notification's identifier and payload are written by whoever
//! published it, which is a different principal from the operator who
//! wrote the trigger template. When the rendered text is handed to a
//! shell or used as a URL, a value containing metacharacters must stay
//! data. This module provides the two neutralisers the template engine
//! applies per [`super::Sink`].
//!
//! # Shell
//!
//! Which characters the shell treats specially depends on where the
//! value lands. [`ShellContext`] follows the operator's literal text the
//! way `sh` would, then quotes each value for the state it is in:
//!
//! | Operator wrote            | State at `{{ }}` | Value `a'b$(x)` becomes   |
//! |---------------------------|------------------|---------------------------|
//! | `run {{ v }}`             | bare             | `'a'\''b$(x)'`            |
//! | `run '{{ v }}'`           | single quotes    | `a'\''b$(x)`              |
//! | `run "{{ v }}"`           | double quotes    | `a'b\$(x)`                |
//!
//! Each row renders to a single argument whose text is exactly the
//! value. Only `'` is special inside single quotes; only `\`, `$`,
//! `` ` `` and `"` are special inside double quotes; a bare value is
//! wrapped in single quotes so nothing in it is special.
//!
//! # URL
//!
//! [`percent_encode`] keeps RFC 3986 unreserved characters and encodes
//! every other byte, so a value cannot add path segments, query
//! parameters or a different host.

/// The shell's quoting state at a point in a command string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellContext {
    /// Outside any quotes.
    Bare,
    /// Inside `'...'`.
    SingleQuoted,
    /// Inside `"..."`.
    DoubleQuoted,
}

impl ShellContext {
    /// Reads operator-written text and returns the state the shell is in
    /// at its end. A backslash outside single quotes escapes the next
    /// character, so `\"` does not open or close double quotes.
    ///
    /// Valid: `echo "` leaves [`Self::DoubleQuoted`]; `echo \"` stays
    /// [`Self::Bare`]. The text is the operator's and is not validated:
    /// an unbalanced quote simply leaves the state open, which is what
    /// the shell would do too.
    pub(super) fn advance(self, literal: &str) -> Self {
        let mut context = self;
        let mut chars = literal.chars();
        while let Some(c) = chars.next() {
            context = match (context, c) {
                (Self::Bare, '\'') => Self::SingleQuoted,
                (Self::Bare, '"') => Self::DoubleQuoted,
                (Self::Bare | Self::DoubleQuoted, '\\') => {
                    chars.next();
                    context
                }
                (Self::SingleQuoted, '\'') | (Self::DoubleQuoted, '"') => Self::Bare,
                _ => context,
            };
        }
        context
    }

    /// Returns `value` written so the shell reads it as literal text in
    /// this context.
    pub(super) fn quote(self, value: &str) -> String {
        match self {
            Self::Bare => format!("'{}'", value.replace('\'', "'\\''")),
            Self::SingleQuoted => value.replace('\'', "'\\''"),
            Self::DoubleQuoted => {
                let mut out = String::with_capacity(value.len());
                for c in value.chars() {
                    if matches!(c, '\\' | '$' | '`' | '"') {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out
            }
        }
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
    fn advance_follows_quotes_and_escapes() {
        assert_eq!(ShellContext::Bare.advance("echo "), ShellContext::Bare);
        assert_eq!(
            ShellContext::Bare.advance("echo '"),
            ShellContext::SingleQuoted
        );
        assert_eq!(
            ShellContext::Bare.advance("echo \""),
            ShellContext::DoubleQuoted
        );
        assert_eq!(ShellContext::Bare.advance("echo \\\""), ShellContext::Bare);
        assert_eq!(
            ShellContext::Bare.advance("echo 'a\"b' "),
            ShellContext::Bare
        );
        assert_eq!(
            ShellContext::Bare.advance("echo \"a'b\" "),
            ShellContext::Bare
        );
        assert_eq!(
            ShellContext::SingleQuoted.advance("\\'"),
            ShellContext::Bare
        );
        assert_eq!(
            ShellContext::DoubleQuoted.advance("\\\""),
            ShellContext::DoubleQuoted
        );
    }

    #[test]
    fn quote_neutralises_metacharacters_per_context() {
        let hostile = "a'b$(x)`y` \"z\" \\ ; &";
        assert_eq!(
            ShellContext::Bare.quote(hostile),
            "'a'\\''b$(x)`y` \"z\" \\ ; &'"
        );
        assert_eq!(
            ShellContext::SingleQuoted.quote(hostile),
            "a'\\''b$(x)`y` \"z\" \\ ; &"
        );
        assert_eq!(
            ShellContext::DoubleQuoted.quote(hostile),
            "a'b\\$(x)\\`y\\` \\\"z\\\" \\\\ ; &"
        );
    }

    #[test]
    fn quote_leaves_plain_values_readable() {
        assert_eq!(ShellContext::Bare.quote("12"), "'12'");
        assert_eq!(ShellContext::SingleQuoted.quote("mars"), "mars");
        assert_eq!(ShellContext::DoubleQuoted.quote("20260101"), "20260101");
    }

    #[test]
    fn percent_encode_keeps_unreserved_and_encodes_the_rest() {
        assert_eq!(percent_encode("mars-2.0_x~"), "mars-2.0_x~");
        assert_eq!(percent_encode("a/b?c=1&d"), "a%2Fb%3Fc%3D1%26d");
        assert_eq!(percent_encode("{\"a\":1}"), "%7B%22a%22%3A1%7D");
        assert_eq!(percent_encode("é"), "%C3%A9");
    }
}
