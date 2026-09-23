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
//! The tracker follows quotes, comments and `$( )` nesting, each `$(`
//! opening a fresh command with its own state. It does not follow
//! here-document bodies, arithmetic expansion or backticks. Once it has
//! seen one of those its state may no longer match the shell's, so the
//! engine refuses to place a notification value after that point rather
//! than guess; the operator reaches the notification through the
//! `AVISO_*` variables there instead.
//!
//! # URL
//!
//! [`percent_encode`] keeps RFC 3986 unreserved characters and encodes
//! every other byte, so a value cannot add path segments or query
//! parameters. Encoding cannot stop a value from being the host when the
//! operator put the placeholder there, so [`UrlTracker`] follows the
//! rendered text and reports whether the path has started; the engine
//! refuses a notification value before that point.

/// The shell's quoting state at one level of command substitution.
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

/// The start of a token that needs one more character to be recognised.
/// Kept across pieces, so the two halves may arrive separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    None,
    /// `$`: a following `(` opens a substitution.
    Dollar,
    /// `$(`: a following `(` makes it arithmetic expansion instead.
    DollarParen,
    /// `<`: a following `<` starts a here-document.
    Less,
}

/// What the tracker is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Token(Token),
    /// A backslash that escapes whatever comes next. If that is a
    /// newline the pair vanishes and the interrupted token resumes.
    Escape(Token),
}

/// Follows a command string the way `sh` reads it.
///
/// The shell removes every unquoted backslash-newline pair before it
/// looks for tokens, so the tracker does the same: such a pair is
/// skipped wherever it appears, including across pieces, and the
/// characters on either side are treated as adjacent. Every `$(` opens
/// a fresh command with its own quoting state, so the states form a
/// stack; the innermost is the one a value lands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellTracker {
    /// One entry per open `$(`, plus the outer command at the bottom.
    levels: Vec<Context>,
    /// True when the next character would start a new word, which is
    /// where a `#` begins a comment.
    word_start: bool,
    /// A token started but not finished, waiting for the next character.
    pending: Pending,
    /// The first construct seen that the tracker does not follow. From
    /// that point its state may not match the shell's, so the engine
    /// refuses to place a notification value.
    unsupported: Option<&'static str>,
}

impl ShellTracker {
    pub(super) fn new() -> Self {
        Self {
            levels: vec![Context::Bare],
            word_start: true,
            pending: Pending::Token(Token::None),
            unsupported: None,
        }
    }

    fn context(&self) -> Context {
        *self.levels.last().unwrap_or(&Context::Bare)
    }

    fn set_context(&mut self, context: Context) {
        if let Some(last) = self.levels.last_mut() {
            *last = context;
        }
    }

    fn flag(&mut self, construct: &'static str) {
        self.unsupported.get_or_insert(construct);
    }

    /// Names why a notification value cannot be placed here, if it
    /// cannot: a here-document (`<<`), arithmetic expansion (`$((`) or
    /// backticks seen earlier, which the tracker does not follow and
    /// after which its state can differ from the shell's; or a `$` or
    /// `$(` immediately before the value, which the value's first
    /// character would complete into an expansion.
    pub(super) fn unsupported(&self) -> Option<&'static str> {
        if matches!(
            self.pending,
            Pending::Token(Token::Dollar | Token::DollarParen)
                | Pending::Escape(Token::Dollar | Token::DollarParen)
        ) && self.context() != Context::SingleQuoted
        {
            return Some("a `$` right before the placeholder");
        }
        self.unsupported
    }

    /// Reads a piece of the rendered command and updates the state. Call
    /// it for every piece in order, the operator's literal text and the
    /// quoted values alike, so the state matches what the shell sees.
    ///
    /// A backslash outside single quotes escapes the next character, so
    /// `\"` does not open or close double quotes. Valid: `echo "` ends in
    /// double quotes; `echo \"` stays bare; `# don't` is a comment, not
    /// an open single quote; `"$(printf 'a"b')"` ends bare, because the
    /// quotes inside the substitution belong to it. The text is not
    /// validated: an unbalanced quote leaves the state open, as it would
    /// for the shell.
    pub(super) fn advance(&mut self, text: &str) {
        for c in text.chars() {
            self.step(c);
        }
    }

    fn step(&mut self, c: char) {
        // Inside single quotes and comments nothing is special except
        // the character that ends them.
        match self.context() {
            Context::SingleQuoted => {
                if c == '\'' {
                    self.set_context(Context::Bare);
                    self.word_start = false;
                }
                return;
            }
            Context::Comment => {
                if c == '\n' {
                    self.set_context(Context::Bare);
                    self.word_start = true;
                }
                return;
            }
            Context::Bare | Context::DoubleQuoted => {}
        }

        // Finish a token that was waiting for this character.
        let token = match std::mem::replace(&mut self.pending, Pending::Token(Token::None)) {
            Pending::Escape(interrupted) => {
                if c == '\n' {
                    // A backslash-newline pair is removed by the shell
                    // and leaves everything as it was, including a token
                    // that was half read when the backslash came.
                    self.pending = Pending::Token(interrupted);
                } else {
                    // Any other escaped character is an ordinary part of
                    // the current word.
                    self.word_start = false;
                }
                return;
            }
            Pending::Token(token) => token,
        };
        if c == '\\' {
            self.pending = Pending::Escape(token);
            return;
        }
        match token {
            Token::Dollar if c == '(' => {
                self.pending = Pending::Token(Token::DollarParen);
                return;
            }
            Token::DollarParen if c == '(' => {
                self.flag("arithmetic expansion");
                return;
            }
            Token::DollarParen => {
                self.levels.push(Context::Bare);
                self.word_start = true;
                // `c` is the first character inside the substitution.
            }
            Token::Less if c == '<' => {
                self.flag("a here-document");
                return;
            }
            Token::None | Token::Dollar | Token::Less => {}
        }

        match c {
            '$' => self.pending = Pending::Token(Token::Dollar),
            '`' => self.flag("backticks"),
            _ if self.context() == Context::DoubleQuoted => {
                if c == '"' {
                    self.set_context(Context::Bare);
                    self.word_start = false;
                }
            }
            '#' if self.word_start => self.set_context(Context::Comment),
            '\'' => self.set_context(Context::SingleQuoted),
            '"' => self.set_context(Context::DoubleQuoted),
            '<' => self.pending = Pending::Token(Token::Less),
            ')' if self.levels.len() > 1 => {
                // Closes a `$(`: the surrounding word continues.
                self.levels.pop();
                self.word_start = false;
            }
            _ => self.word_start = matches!(c, ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')'),
        }
    }

    /// Returns `value` written so the shell reads it as literal text in
    /// the current state.
    ///
    /// When the text so far ends with a backslash, the quoted value starts
    /// with a newline: the shell removes a backslash-newline pair, so the
    /// escape is spent on nothing and the quoting that follows is read as
    /// written.
    pub(super) fn quote(&self, value: &str) -> String {
        let quoted = self.quote_in_context(value);
        if matches!(self.pending, Pending::Escape(_)) && self.context() != Context::SingleQuoted {
            format!("\n{quoted}")
        } else {
            quoted
        }
    }

    fn quote_in_context(&self, value: &str) -> String {
        match self.context() {
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
        tracker.context()
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
        // A `)` that closes a `$(` continues the word; one that closes a
        // subshell ends it.
        assert_eq!(after("$(printf x)# don't"), Context::SingleQuoted);
        assert_eq!(after("$(a $(b))# don't"), Context::SingleQuoted);
        assert_eq!(after("(printf x)# don't"), Context::Comment);
        assert_eq!(after("$(# it's\nprintf x) '"), Context::SingleQuoted);
        // Quotes inside a `$( )` belong to it, even inside double quotes.
        assert_eq!(after("echo \"$(printf '%s' 'a\"b')\" "), Context::Bare);
        assert_eq!(after("echo \"$(printf '%s' 'a\"b')"), Context::DoubleQuoted);
        // A backslash-newline is removed by the shell: the word state from
        // before it decides whether the `#` that follows is a comment.
        assert_eq!(after("echo foo \\\n# don't\n'"), Context::SingleQuoted);
        // Without the space the shell sees `foo#don't`: no comment, and the
        // apostrophe opens a quote.
        assert_eq!(after("echo foo\\\n#don't"), Context::SingleQuoted);
        assert_eq!(after("echo foo\\\n#dont"), Context::Bare);
    }

    #[test]
    fn constructs_the_tracker_does_not_follow_are_reported() {
        let seen = |text: &str| {
            let mut tracker = ShellTracker::new();
            tracker.advance(text);
            tracker.unsupported()
        };
        assert_eq!(seen("cat <<EOF\ncan't\nEOF\n"), Some("a here-document"));
        assert_eq!(seen("echo $((1))# "), Some("arithmetic expansion"));
        assert_eq!(seen("echo \"$((1))\" "), Some("arithmetic expansion"));
        assert_eq!(seen("x=`date`"), Some("backticks"));
        assert_eq!(seen("echo \"`date`\""), Some("backticks"));
        assert_eq!(seen("echo $(date) '<<' '$((' \"<<\" "), None);
    }

    #[test]
    fn advance_over_a_quoted_value_returns_to_the_surrounding_state() {
        let mut tracker = ShellTracker::new();
        tracker.advance("run ");
        tracker.advance(&tracker.quote("a'b\"c"));
        assert_eq!(tracker.context(), Context::Bare);
        tracker.advance(" '");
        tracker.advance(&tracker.quote("it's"));
        assert_eq!(tracker.context(), Context::SingleQuoted);
        tracker.advance("' \"");
        tracker.advance(&tracker.quote("say \"hi\" $x"));
        assert_eq!(tracker.context(), Context::DoubleQuoted);
    }

    #[test]
    fn a_trailing_backslash_is_spent_before_the_value() {
        // An operator's env value ends with a backslash. Without care the
        // opening quote of the next value would be escaped and the value
        // read bare by the shell.
        let mut tracker = ShellTracker::new();
        tracker.advance("run C:\\dir\\");
        assert_eq!(tracker.pending, Pending::Escape(Token::None));
        let quoted = tracker.quote("a'b$(x)");
        assert_eq!(quoted, "\n'a'\\''b$(x)'");
        tracker.advance(&quoted);
        assert_eq!(tracker.context(), Context::Bare);
        assert_eq!(tracker.pending, Pending::Token(Token::None));

        let mut tracker = ShellTracker::new();
        tracker.advance("run \"x\\");
        let quoted = tracker.quote("$(y)");
        assert_eq!(quoted, "\n\\$(y)");
        tracker.advance(&quoted);
        assert_eq!(tracker.context(), Context::DoubleQuoted);

        // An env value that starts with a newline right after a trailing
        // backslash: sh removes the pair and the word state from before
        // it decides whether the `#` that follows is a comment.
        let mut tracker = ShellTracker::new();
        tracker.advance("echo foo \\");
        tracker.advance("\n# don't\n'");
        assert_eq!(tracker.context(), Context::SingleQuoted);
        let mut tracker = ShellTracker::new();
        tracker.advance("echo foo\\");
        tracker.advance("\n#don't");
        assert_eq!(tracker.context(), Context::SingleQuoted);
    }

    #[test]
    fn a_trailing_dollar_cannot_join_the_value_into_an_expansion() {
        // `"${{ v }}"` with a value of `(touch x)` would render
        // `"$(touch x)"`, so a value right after a `$` is refused.
        let mut tracker = ShellTracker::new();
        tracker.advance("printf '%s' \"$");
        assert_eq!(
            tracker.unsupported(),
            Some("a `$` right before the placeholder")
        );
        let mut tracker = ShellTracker::new();
        tracker.advance("run $");
        assert_eq!(
            tracker.unsupported(),
            Some("a `$` right before the placeholder")
        );
        // Inside single quotes a `$` is literal and the value is fine.
        let mut tracker = ShellTracker::new();
        tracker.advance("run '$");
        assert_eq!(tracker.unsupported(), None);
        // Once more text follows, the `$` was a literal dollar sign.
        let mut tracker = ShellTracker::new();
        tracker.advance("run $");
        tracker.advance("x ");
        assert_eq!(tracker.unsupported(), None);

        // A token split across pieces, or across a backslash-newline,
        // is read the way the shell reads it: as one token.
        let mut tracker = ShellTracker::new();
        tracker.advance("run $");
        tracker.advance("(printf '%s' 'a\"b')# don't");
        assert_eq!(tracker.context(), Context::SingleQuoted);
        let mut tracker = ShellTracker::new();
        tracker.advance("run $");
        tracker.advance("((1))");
        assert_eq!(tracker.unsupported(), Some("arithmetic expansion"));
        let mut tracker = ShellTracker::new();
        tracker.advance("run $(");
        tracker.advance("(1))");
        assert_eq!(tracker.unsupported(), Some("arithmetic expansion"));
        let mut tracker = ShellTracker::new();
        tracker.advance("x=$\\\n(echo a)# don't");
        assert_eq!(tracker.context(), Context::SingleQuoted);
        let mut tracker = ShellTracker::new();
        tracker.advance("cat <");
        tracker.advance("<EOF\n");
        assert_eq!(tracker.unsupported(), Some("a here-document"));
        let mut tracker = ShellTracker::new();
        tracker.advance("cat <\\\n<EOF\n");
        assert_eq!(tracker.unsupported(), Some("a here-document"));
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
