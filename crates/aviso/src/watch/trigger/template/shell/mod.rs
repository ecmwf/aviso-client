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
//! `AVISO_*` variables there instead. Quoting also only holds for one
//! read, so a value is refused where the command name goes, in a
//! redirection operand, and in the arguments of `eval`, `trap` or a
//! shell run with `-c`, which read their arguments as code again.

mod level;
mod quote;
mod state;
#[cfg(test)]
mod tests;

use level::{Level, Opener, Word};
use state::{Context, Pending, Token};

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
    /// The word being read at the current level, while bare.
    word: Word,
    /// True while the word being read is the first word of a command at
    /// the current level: a value placed there would choose the program
    /// to run.
    command_word: bool,
    /// True once the command at the current level is one that reads its
    /// arguments as shell code again (`eval`, `trap`, `sh -c`), or one
    /// whose name is assembled from an expansion and so cannot be told
    /// apart from those. A value anywhere in such a command is refused.
    reparsing_command: bool,
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
            levels: vec![Level {
                context: Context::Bare,
                opener: Opener::None,
                outer_command_word: false,
                outer_reparsing: false,
            }],
            word_start: true,
            word: Word::default(),
            command_word: true,
            reparsing_command: false,
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
        // The word this expansion sits in continues after it, assembled
        // from parts the tracker cannot see. If that word names the
        // command, the program is unknown, and so is whether it reparses
        // its arguments: refuse values in that command.
        if self.command_word && opener != Opener::Group {
            self.reparsing_command = true;
        }
        self.levels.push(Level {
            context: Context::Bare,
            opener,
            outer_command_word: self.command_word,
            outer_reparsing: self.reparsing_command,
        });
        self.word_start = true;
        self.word.clear();
        self.command_word = opener != Opener::Brace;
        self.reparsing_command = false;
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
            self.reparsing_command = level.outer_reparsing;
            if level.opener != Opener::Group {
                self.word.has_expansion = true;
            }
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
        self.word.push(c);
    }

    /// Closes the current word. The word `case` is flagged wherever it
    /// stands: a `case` statement's pattern list has `)` without a
    /// matching `(`, which the tracker does not follow, and the shell
    /// grammar that decides when `case` is a keyword is not worth
    /// modelling when refusing is free.
    fn end_word(&mut self, separator_starts_command: bool) {
        if self.word.text == "case" {
            self.flag("a case statement");
        }
        // A redirection operand is not a command word and does not take
        // the command position: `> out cmd` still runs `cmd`.
        if !self.word.is_empty()
            && !self.word.redirect_operand
            && !Self::leaves_command_position_open(&self.word)
        {
            if self.command_word && Self::reparses_its_arguments(&self.word.text) {
                self.reparsing_command = true;
            }
            self.command_word = false;
        }
        self.word.clear();
        if separator_starts_command {
            self.command_word = true;
            self.reparsing_command = false;
        }
    }

    /// True for a command that reads its arguments as shell code a second
    /// time, so quoting done for the first read does not hold: `eval`,
    /// `trap`, and a shell started with `-c`. The shells are listed by
    /// name; `sh -c` is the shape every one of them takes.
    fn reparses_its_arguments(word: &str) -> bool {
        matches!(
            word,
            "eval" | "trap" | "sh" | "bash" | "dash" | "zsh" | "ksh" | "ash" | "busybox"
        )
    }

    /// True for a first word after which the next word is still the
    /// command name: an assignment such as `MODE=prod`, the negation `!`,
    /// the reserved words that introduce a command, and the utilities
    /// that run their first argument (`command`, `exec`, `nohup`, `env`,
    /// `nice`, `time`, `xargs`, `sudo`, `timeout`). Their options are
    /// covered by the leading `-`, and their numeric arguments, such as
    /// the seconds of `timeout`, by the leading digit. An option that
    /// takes a separate value, such as `sudo -u name`, is not: that name
    /// is taken for the command, and a value after it is accepted as an
    /// argument. Operators pass the notification to such wrappers through
    /// the `AVISO_*` variables, which the docs recommend for anything
    /// beyond a plain command word.
    fn leaves_command_position_open(word: &Word) -> bool {
        matches!(
            word.text.as_str(),
            "!" | "{"
                | "if"
                | "then"
                | "else"
                | "elif"
                | "while"
                | "until"
                | "do"
                | "time"
                | "command"
                | "exec"
                | "builtin"
                | "nohup"
                | "env"
                | "nice"
                | "xargs"
                | "sudo"
                | "doas"
                | "timeout"
                | "stdbuf"
                | "setsid"
                | "chroot"
                | "flock"
                | "watch"
        ) || word.assignment
            || word
                .text
                .starts_with(|c: char| c == '-' || c.is_ascii_digit())
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
        if self.context() != Context::Comment {
            if self.word.redirect_operand {
                return Some("a redirection");
            }
            if self.command_word && self.word.is_empty() {
                return Some("the command word");
            }
            if self.reparsing_command {
                return Some("an argument to a command the engine cannot name");
            }
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
                    self.command_word = true;
                    self.reparsing_command = false;
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
                // first character of that word arrives; the whole word
                // is then the operand, and a value anywhere in it is
                // refused.
                if matches!(c, '>' | ' ' | '\t') {
                    self.pending = Pending::Token(Token::Redirect);
                    return;
                }
                self.word.clear();
                self.word.redirect_operand = true;
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
}
