// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The quoting state of one level and the tokens that wait for their
//! next character.

/// The shell's quoting state at one level of command substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Context {
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
pub(super) enum Token {
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
pub(super) enum Pending {
    Token(Token),
    /// A backslash that escapes whatever comes next. If that is a
    /// newline the pair vanishes and the interrupted token resumes.
    Escape(Token),
}
