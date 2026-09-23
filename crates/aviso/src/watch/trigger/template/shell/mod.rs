// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Neutralises notification values for a `/bin/sh -c` command.
//!
//! A notification's identifier and payload are written by whoever
//! published it, which is a different principal from the operator who
//! wrote the trigger template. When the rendered text is handed to a
//! shell, a value containing metacharacters must stay data.
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
//! The tracker follows quotes, comments, and `$( )` and `( )` nesting,
//! each opening a fresh command with its own state. It does not follow
//! here-document bodies, arithmetic expansion, backticks or `case`
//! statements, whose pattern list has unmatched `)`. Once it has
//! seen one of those its state may no longer match the shell's, so the
//! engine refuses to place a notification value after that point rather
//! than guess; the operator reaches the notification through the
//! `AVISO_*` variables there instead.

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
    /// A redirection operator: the next word names a file.
    Redirect,
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
/// characters on either side are treated as adjacent. Every `$(` and
/// every `(` opens a fresh command with its own quoting state, so the
/// states form a stack; the innermost is the one a value lands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellTracker {
    /// One entry per open `$(` or `(`, plus the outer command at the
    /// bottom.
    levels: Vec<Level>,
    /// True when the next character would start a new word, which is
    /// where a `#` begins a comment.
    word_start: bool,
    /// The word being read at the current level, while bare. Only its
    /// first few characters matter, to recognise `case`.
    word: String,
    /// True while the word being read is the first word of a command at
    /// the current level: a value placed there would choose the program
    /// to run.
    command_word: bool,
    /// A token started but not finished, waiting for the next character.
    pending: Pending,
    /// The first construct seen that the tracker does not follow. From
    /// that point its state may not match the shell's, so the engine
    /// refuses to place a notification value.
    unsupported: Option<&'static str>,
}

/// What kind of bracket opened a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opener {
    /// The outer command.
    None,
    /// `$( )`: its `)` continues the word around the substitution.
    Substitution,
    /// `( )`: its `)` ends a word.
    Group,
    /// `${ }`: closed by `}`; a `)` inside is ordinary text.
    Brace,
}

/// One open command or expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Level {
    context: Context,
    opener: Opener,
    /// Whether the word this level was opened in was the command word of
    /// the level outside, restored when this level closes so `$(x)` at
    /// the start of a command still leaves what follows as its name.
    outer_command_word: bool,
}

impl ShellTracker {
    pub(super) fn new() -> Self {
        Self {
            levels: vec![Level {
                context: Context::Bare,
                opener: Opener::None,
                outer_command_word: false,
            }],
            word_start: true,
            word: String::new(),
            command_word: true,
            pending: Pending::Token(Token::None),
            unsupported: None,
        }
    }

    fn context(&self) -> Context {
        self.levels
            .last()
            .map_or(Context::Bare, |level| level.context)
    }

    fn set_context(&mut self, context: Context) {
        if let Some(last) = self.levels.last_mut() {
            last.context = context;
        }
    }

    fn open_level(&mut self, opener: Opener) {
        self.levels.push(Level {
            context: Context::Bare,
            opener,
            outer_command_word: self.command_word,
        });
        self.word_start = true;
        self.word.clear();
        self.command_word = opener != Opener::Brace;
    }

    /// Closes the innermost level, if it is not the outer command, and
    /// restores whether the word it sat in names the command.
    fn close_level(&mut self) -> Option<Level> {
        if self.levels.len() <= 1 {
            return None;
        }
        let closed = self.levels.pop();
        if let Some(level) = closed {
            self.command_word = level.outer_command_word;
        }
        closed
    }

    fn opener(&self) -> Opener {
        self.levels
            .last()
            .map_or(Opener::None, |level| level.opener)
    }

    /// Records a bare character as part of the current word.
    fn note_word_char(&mut self, c: char) {
        if self.word_start {
            self.word.clear();
        }
        if self.word.len() < 5 {
            self.word.push(c);
        }
    }

    /// Closes the current word. The word `case` is flagged wherever it
    /// stands: a `case` statement's pattern list has `)` without a
    /// matching `(`, which the tracker does not follow, and the shell
    /// grammar that decides when `case` is a keyword is not worth
    /// modelling when refusing is free.
    fn end_word(&mut self, separator_starts_command: bool) {
        if self.word == "case" {
            self.flag("a case statement");
        }
        if !self.word.is_empty() {
            self.command_word = false;
        }
        self.word.clear();
        if separator_starts_command {
            self.command_word = true;
        }
    }

    fn flag(&mut self, construct: &'static str) {
        self.unsupported.get_or_insert(construct);
    }

    /// Names why a notification value cannot be placed here, if it
    /// cannot: a here-document (`<<`), arithmetic expansion (`$((`),
    /// backticks or `case` seen earlier, which the tracker does not
    /// follow and after which its state can differ from the shell's; a
    /// `$` or `$(` immediately before the value, which the value's first
    /// character would complete into an expansion; an open `${ }`; or
    /// the value standing where the command name goes, which would let
    /// the publisher choose the program.
    pub(super) fn unsupported(&self) -> Option<&'static str> {
        if self.unsupported.is_some() {
            return self.unsupported;
        }
        if matches!(
            self.pending,
            Pending::Token(Token::Dollar | Token::DollarParen)
                | Pending::Escape(Token::Dollar | Token::DollarParen)
        ) && self.context() != Context::SingleQuoted
        {
            return Some("a `$` right before the placeholder");
        }
        if matches!(
            self.pending,
            Pending::Token(Token::Less | Token::Redirect)
                | Pending::Escape(Token::Less | Token::Redirect)
        ) && self.context() != Context::SingleQuoted
        {
            return Some("a redirection");
        }
        if self.opener() == Opener::Brace {
            return Some("an open `${ }` expansion");
        }
        if self.command_word && self.word.is_empty() && self.context() != Context::Comment {
            return Some("the command word");
        }
        None
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
                // `c` is the first character inside the substitution.
                self.open_level(Opener::Substitution);
            }
            Token::Dollar if c == '{' => {
                // Shells disagree on how quotes inside a `${ }` that sits
                // inside double quotes are read, so that shape is not
                // followed.
                if self.context() == Context::DoubleQuoted {
                    self.flag("a `${ }` expansion inside double quotes");
                }
                self.open_level(Opener::Brace);
                return;
            }
            Token::Less if c == '<' => {
                self.flag("a here-document");
                return;
            }
            Token::Less | Token::Redirect => {
                // A redirection operator, `<`, `>`, `>>` or `<>`, is
                // followed by a file name. It stays pending until the
                // first character of that word arrives, so a value
                // placed there is refused.
                if matches!(c, '>' | ' ' | '\t') {
                    self.pending = Pending::Token(Token::Redirect);
                    return;
                }
                self.word_start = false;
            }
            Token::None | Token::Dollar => {}
        }

        self.step_char(c);
    }

    /// Handles one ordinary character in bare or double-quoted text, once
    /// pending tokens have been dealt with.
    fn step_char(&mut self, c: char) {
        match c {
            '$' => {
                self.pending = Pending::Token(Token::Dollar);
                self.word_start = false;
            }
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
            '<' => {
                self.pending = Pending::Token(Token::Less);
                self.word_start = false;
            }
            '>' => {
                self.pending = Pending::Token(Token::Redirect);
                self.word_start = false;
            }
            _ if self.opener() == Opener::Brace => {
                // Inside `${ }` only the closing brace matters; the word
                // around the expansion continues after it.
                if c == '}' {
                    self.close_level();
                    self.word_start = false;
                }
            }
            '(' => {
                self.end_word(true);
                self.open_level(Opener::Group);
            }
            ')' => {
                self.end_word(true);
                let closed = self.close_level();
                // The `)` of a group ends a word and the next word starts
                // a command; the `)` of a `$( )` continues the word around
                // the substitution, which keeps whatever role it had.
                self.word_start = closed.is_none_or(|level| level.opener == Opener::Group);
                if self.word_start {
                    self.command_word = true;
                }
            }
            ' ' | '\t' => {
                self.end_word(false);
                self.word_start = true;
            }
            '\n' | ';' | '&' | '|' => {
                self.end_word(true);
                self.word_start = true;
            }
            _ => {
                self.note_word_char(c);
                self.word_start = false;
            }
        }
    }

    /// Reads a quoted notification value. Its text is not a word start
    /// and never a command name; the tracker only needs to know it is
    /// no longer at the start of a word.
    pub(super) fn advance_value(&mut self, quoted: &str) {
        self.advance(quoted);
        self.command_word = false;
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

#[cfg(test)]
mod tests;
