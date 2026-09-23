// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Writing a value so the shell reads it as literal text in the state
//! the tracker is in.

use super::ShellTracker;
use super::state::{Context, Pending};

impl ShellTracker {
    /// Returns `value` written so the shell reads it as literal text in
    /// the current state.
    ///
    /// When the text so far ends with a backslash, the quoted value starts
    /// with a newline: the shell removes a backslash-newline pair, so the
    /// escape is spent on nothing and the quoting that follows is read as
    /// written.
    pub(in crate::watch::trigger::template) fn quote(&self, value: &str) -> String {
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
