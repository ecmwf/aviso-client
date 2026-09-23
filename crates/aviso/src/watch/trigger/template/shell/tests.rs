// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Unit tests for the shell tracker.

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
    // `$#` is a parameter, not a comment; a `(` inside `$( )` opens a
    // group whose `)` does not close the substitution.
    assert_eq!(after("printf '%s' $# '"), Context::SingleQuoted);
    assert_eq!(after("echo $( (printf x) )# don't"), Context::SingleQuoted);
    assert_eq!(after("(a; (b))# don't"), Context::Comment);
    // A `)` inside `${ }` is text; the `}` closes the expansion.
    assert_eq!(
        after("echo \"$(printf '%s' ${x:-foo)}'a')\""),
        Context::Bare
    );
    assert_eq!(after("echo ${x:-a)b}'"), Context::SingleQuoted);
    // A redirection operator does not change the quoting state.
    assert_eq!(
        after("printf ok > /dev/null; printf '"),
        Context::SingleQuoted
    );
    assert_eq!(after("printf ok 2>&1 >out '"), Context::SingleQuoted);
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
    assert_eq!(seen("case x in a) echo;; esac"), Some("a case statement"));
    assert_eq!(
        seen("$(if :; then case x in a) :;; esac; fi)# "),
        Some("a case statement")
    );
    // Flagged wherever the bare word appears; the shell grammar is not
    // modelled and refusing costs only an error message.
    assert_eq!(seen("echo case "), Some("a case statement"));
    assert_eq!(seen("showcase; echo 'case' \"case\" "), None);
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
fn a_value_cannot_be_the_command_word_or_sit_inside_a_brace_expansion() {
    let refused = |text: &str| {
        let mut tracker = ShellTracker::new();
        tracker.advance(text);
        tracker.unsupported()
    };
    assert_eq!(refused(""), Some("the command word"));
    assert_eq!(refused("run x; "), Some("the command word"));
    assert_eq!(refused("echo $(printf x; "), Some("the command word"));
    assert_eq!(refused("x=${y:-"), Some("an open `${ }` expansion"));
    assert_eq!(
        refused("printf '%s' \"${UNSET:-x}\" "),
        Some("a `${ }` expansion inside double quotes")
    );
    assert_eq!(refused("printf ok > "), Some("a redirection"));
    assert_eq!(refused("printf ok >"), Some("a redirection"));
    assert_eq!(refused("printf ok >> "), Some("a redirection"));
    assert_eq!(refused("printf ok 2> "), Some("a redirection"));
    assert_eq!(refused("cat < "), Some("a redirection"));
    assert_eq!(refused("printf ok > /dev/null "), None);
    assert_eq!(refused("printf ok >/dev/null; printf "), None);
    // Assignments, `!` and the reserved words do not take the command
    // position; the word after them names the command.
    for prefix in [
        "MODE=prod ",
        "! ",
        "if ",
        "while ",
        "{ ",
        "A=1 B=2 ",
        "if ! ",
        "command ",
        "exec ",
        "nohup ",
        "env -i ",
        "timeout 5 ",
        "sudo ",
        "xargs -0 ",
    ] {
        assert_eq!(refused(prefix), Some("the command word"), "{prefix:?}");
    }
    // Long names still count as assignments; the `=` is kept as a fact
    // about the word, not found in its stored prefix.
    assert_eq!(refused("VERY_LONG_NAME=prod "), Some("the command word"));
    // A redirection operand does not take the command position, and a
    // value anywhere in that word, not only at its start, is refused.
    assert_eq!(refused("> /tmp/out "), Some("the command word"));
    assert_eq!(refused("> /safe/"), Some("a redirection"));
    assert_eq!(refused("2>/var/log/x."), Some("a redirection"));
    // A comment ends the command; the next line starts a new one.
    assert_eq!(refused("printf ok # note\n"), Some("the command word"));
    assert_eq!(refused("MODE=prod run "), None);
    assert_eq!(refused("if run "), None);
    assert_eq!(refused("exec run "), None);
    assert_eq!(refused("timeout 5 run "), None);
    // An expansion at the start of a command is still part of its name.
    assert_eq!(refused("$(true)"), Some("the command word"));
    assert_eq!(refused("${CMD}"), Some("the command word"));
    assert_eq!(refused("run $(true)"), None);
    assert_eq!(refused("(true)"), Some("the command word"));
    assert_eq!(refused("run "), None);
    assert_eq!(refused("run '"), None);
    assert_eq!(refused("echo $(printf "), None);
    assert_eq!(refused("# "), None);
    // A value in argument position, then another command: the second
    // command word is the operator's.
    let mut tracker = ShellTracker::new();
    tracker.advance("run ");
    tracker.advance_value(&tracker.quote("v"));
    tracker.advance("; printf ");
    assert_eq!(tracker.unsupported(), None);
}

#[test]
fn a_value_cannot_be_an_argument_to_a_command_that_reparses_it() {
    let refused = |text: &str| {
        let mut tracker = ShellTracker::new();
        tracker.advance(text);
        tracker.unsupported()
    };
    let reason = Some("an argument to a command that reads it as shell code");
    assert_eq!(refused("eval '"), reason);
    assert_eq!(refused("trap '"), reason);
    assert_eq!(refused("trap \""), reason);
    assert_eq!(refused("sh -c '"), reason);
    assert_eq!(refused("/bin/bash -c \""), reason);
    assert_eq!(refused("exec sh -c '"), reason);
    assert_eq!(refused("x=$(sh -c '"), reason);
    // The flag belongs to the command; the next command is clean, and a
    // substitution inside a reparsing command is its own command.
    assert_eq!(refused("eval 'x'; run "), None);
    assert_eq!(refused("trap 'x' EXIT\nrun "), None);
    assert_eq!(refused("sh -c \"$(run "), None);
    // `sh` as an argument is just a word.
    assert_eq!(refused("run sh "), None);
}
