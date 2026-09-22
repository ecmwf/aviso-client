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
//! value lands. [`ShellTracker`] reads the rendered text as it is
//! produced, values included, and keeps the quoting state `sh` would be
//! in. Each value is then quoted for that state:
//!
//! | Operator wrote            | State at `{{ }}` | Value `a'b$(x)` becomes   |
//! |---------------------------|------------------|---------------------------|
//! | `run {{ v }}`             | bare             | `'a'\''b$(x)'`            |
//! | `run '{{ v }}'`           | single quotes    | `a'\''b$(x)`              |
//! | `run "{{ v }}"`           | double quotes    | `a'b\$(x)`                |
//! | `# {{ v }}`               | comment          | `a'b$(x)`                 |
//!
//! Each of the first three rows renders to a single argument whose text
//! is exactly the value. Only `'` is special inside single quotes; only
//! `\`, `$`, `` ` `` and `"` are special inside double quotes; a bare
//! value is wrapped in single quotes so nothing in it is special. A
//! comment runs to the end of the line and nothing in it is read, so a
//! value there is left alone; tracking comments matters because an
//! apostrophe in one must not be mistaken for an opening quote.
//!
//! The tracker models a command word in those three states. It does not
//! model the inside of backticks, here-document bodies, or the text an
//! operator passes to `eval`; a value placed there is not protected, and
//! the documentation says so.
//!
//! # URL
//!
//! [`percent_encode`] keeps RFC 3986 unreserved characters and encodes
//! every other byte, so a value cannot add path segments or query
//! parameters. Encoding cannot stop a value from being the host when the
//! operator put the placeholder there, so [`UrlTracker`] follows the
//! rendered text and reports whether the path has started; the engine
//! refuses a notification value before that point.

/// The shell's quoting state at a point in a command string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// Outside any quotes.
    Bare,
    /// Inside `'...'`.
    SingleQuoted,
    /// Inside `"..."`.
    DoubleQuoted,
    /// After a `#` that starts a word, until the end of the line.
    Comment,
}

/// Follows a command string the way `sh` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ShellTracker {
    context: Context,
    /// True when the next character would start a new word, which is
    /// where a `#` begins a comment.
    word_start: bool,
}

impl ShellTracker {
    pub(super) fn new() -> Self {
        Self {
            context: Context::Bare,
            word_start: true,
        }
    }

    /// Reads a piece of the rendered command and updates the state. Call
    /// it for every piece in order, the operator's literal text and the
    /// quoted values alike, so the state matches what the shell sees.
    ///
    /// A backslash outside single quotes escapes the next character, so
    /// `\"` does not open or close double quotes. Valid: `echo "` ends in
    /// double quotes; `echo \"` stays bare; `# don't` is a comment, not
    /// an open single quote. The text is not validated: an unbalanced
    /// quote leaves the state open, as it would for the shell.
    pub(super) fn advance(&mut self, text: &str) {
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            match self.context {
                Context::Comment => {
                    if c == '\n' {
                        self.context = Context::Bare;
                        self.word_start = true;
                    }
                }
                Context::Bare => {
                    match c {
                        '#' if self.word_start => self.context = Context::Comment,
                        '\'' => self.context = Context::SingleQuoted,
                        '"' => self.context = Context::DoubleQuoted,
                        '\\' => {
                            chars.next();
                        }
                        _ => {}
                    }
                    self.word_start = self.context == Context::Bare
                        && matches!(c, ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')');
                }
                Context::SingleQuoted => {
                    if c == '\'' {
                        self.context = Context::Bare;
                        self.word_start = false;
                    }
                }
                Context::DoubleQuoted => match c {
                    '"' => {
                        self.context = Context::Bare;
                        self.word_start = false;
                    }
                    '\\' => {
                        chars.next();
                    }
                    _ => {}
                },
            }
        }
    }

    /// Returns `value` written so the shell reads it as literal text in
    /// the current state.
    pub(super) fn quote(self, value: &str) -> String {
        match self.context {
            Context::Bare => format!("'{}'", value.replace('\'', "'\\''")),
            Context::SingleQuoted => value.replace('\'', "'\\''"),
            Context::DoubleQuoted => {
                let mut out = String::with_capacity(value.len());
                for c in value.chars() {
                    if matches!(c, '\\' | '$' | '`' | '"') {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out
            }
            // Nothing in a comment is read. A newline in the value would
            // end the comment, so it is dropped.
            Context::Comment => value.replace('\n', " "),
        }
    }
}

/// Which part of a URL the rendered text has reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UrlTracker {
    seen_scheme_separator: bool,
    in_path: bool,
}

impl UrlTracker {
    pub(super) fn new() -> Self {
        Self {
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
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() && !self.in_path {
            if self.seen_scheme_separator {
                self.in_path = matches!(bytes[i], b'/' | b'?' | b'#');
                i += 1;
            } else if bytes[i..].starts_with(b"://") {
                self.seen_scheme_separator = true;
                i += 3;
            } else {
                i += 1;
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

    fn after(text: &str) -> Context {
        let mut tracker = ShellTracker::new();
        tracker.advance(text);
        tracker.context
    }

    #[test]
    fn advance_follows_quotes_and_escapes() {
        assert_eq!(after("echo "), Context::Bare);
        assert_eq!(after("echo '"), Context::SingleQuoted);
        assert_eq!(after("echo \""), Context::DoubleQuoted);
        assert_eq!(after("echo \\\""), Context::Bare);
        assert_eq!(after("echo 'a\"b' "), Context::Bare);
        assert_eq!(after("echo \"a'b\" "), Context::Bare);
        assert_eq!(after("'\\'"), Context::Bare);
        assert_eq!(after("\"\\\""), Context::DoubleQuoted);
    }

    #[test]
    fn advance_knows_where_a_comment_starts_and_ends() {
        assert_eq!(after("# don't\n"), Context::Bare);
        assert_eq!(after("# don't"), Context::Comment);
        assert_eq!(after("run x # it's\n"), Context::Bare);
        assert_eq!(after("run;# it's\n"), Context::Bare);
        // A `#` inside a word, or right after a quote, is not a comment.
        assert_eq!(after("run a#b'"), Context::SingleQuoted);
        assert_eq!(after("run 'x'#it's"), Context::SingleQuoted);
        assert_eq!(after("echo '#' '"), Context::SingleQuoted);
    }

    #[test]
    fn advance_over_a_quoted_value_returns_to_the_surrounding_state() {
        let mut tracker = ShellTracker::new();
        tracker.advance("run ");
        tracker.advance(&tracker.quote("a'b\"c"));
        assert_eq!(tracker.context, Context::Bare);
        tracker.advance(" '");
        tracker.advance(&tracker.quote("it's"));
        assert_eq!(tracker.context, Context::SingleQuoted);
        tracker.advance("' \"");
        tracker.advance(&tracker.quote("say \"hi\" $x"));
        assert_eq!(tracker.context, Context::DoubleQuoted);
    }

    #[test]
    fn quote_neutralises_metacharacters_per_context() {
        let hostile = "a'b$(x)`y` \"z\" \\ ; &";
        let mut tracker = ShellTracker::new();
        assert_eq!(tracker.quote(hostile), "'a'\\''b$(x)`y` \"z\" \\ ; &'");
        tracker.advance("'");
        assert_eq!(tracker.quote(hostile), "a'\\''b$(x)`y` \"z\" \\ ; &");
        tracker.advance("' \"");
        assert_eq!(
            tracker.quote(hostile),
            "a'b\\$(x)\\`y\\` \\\"z\\\" \\\\ ; &"
        );
        tracker.advance("\" # ");
        assert_eq!(tracker.quote("x\ny"), "x y");
    }

    #[test]
    fn quote_leaves_plain_values_readable() {
        let mut tracker = ShellTracker::new();
        assert_eq!(tracker.quote("12"), "'12'");
        tracker.advance("'");
        assert_eq!(tracker.quote("mars"), "mars");
        tracker.advance("' \"");
        assert_eq!(tracker.quote("20260101"), "20260101");
    }

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
    }

    #[test]
    fn percent_encode_keeps_unreserved_and_encodes_the_rest() {
        assert_eq!(percent_encode("mars-2.0_x~"), "mars-2.0_x~");
        assert_eq!(percent_encode("a/b?c=1&d"), "a%2Fb%3Fc%3D1%26d");
        assert_eq!(percent_encode("{\"a\":1}"), "%7B%22a%22%3A1%7D");
        assert_eq!(percent_encode("é"), "%C3%A9");
    }
}
