// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The pieces of tracker state that describe where in a command the
//! text currently is: the open levels and the word being read.

use super::state::Context;

/// What kind of bracket opened a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Opener {
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
pub(super) struct Level {
    pub(super) context: Context,
    pub(super) opener: Opener,
    /// Whether the word this level was opened in was the command word of
    /// the level outside, restored when this level closes so `$(x)` at
    /// the start of a command still leaves what follows as its name.
    pub(super) outer_command_word: bool,
    /// Whether the command this level was opened in reparses its
    /// arguments, restored likewise.
    pub(super) outer_reparsing: bool,
}

/// The bare word being read, with the facts about it the tracker acts
/// on. Only a prefix of the text after its last `/` is kept, enough to
/// recognise `case`, the words that leave the command position open and
/// the shells; the flags are exact whatever the length.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Word {
    /// The first characters of the word.
    pub(super) text: String,
    /// True when the word contains `=` outside quotes, which makes it
    /// an assignment when it stands first.
    pub(super) assignment: bool,
    /// True when the word names the file of a redirection.
    pub(super) redirect_operand: bool,
}

impl Word {
    pub(super) fn push(&mut self, c: char) {
        if c == '=' {
            self.assignment = true;
        }
        if c == '/' {
            // Only the last path segment names the program.
            self.text.clear();
            return;
        }
        if self.text.len() < 8 {
            self.text.push(c);
        }
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.text.is_empty() && !self.assignment && !self.redirect_operand
    }
}
