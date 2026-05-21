//! Stdout output helpers obeying the workspace's no-`println!` rule.
//!
//! The workspace clippy lints set `print_stdout = "deny"` and
//! `print_stderr = "deny"` so neither `println!` nor `eprintln!` is
//! usable in CLI code. Every operator-facing stdout write goes
//! through [`write_stdout_line`] or [`write_stdout_bytes`], which
//! acquire a single locked [`std::io::Stdout`] guard and call
//! [`std::io::Write::write_all`] (the named-data-output pattern the
//! Echo trigger uses; the lint permits `write_all` against an
//! explicit handle, only the `println!` macro is rejected).
//!
//! [`is_stdout_tty`] wraps `std::io::IsTerminal` so subcommands
//! pick the TTY-aware text form versus the NDJSON pipe form per
//! Q5; the `--json` flag overrides this to always pick JSON via
//! [`use_ndjson`].

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result};

/// Writes `line` to stdout followed by a newline.
///
/// Builds an owned `Vec<u8>` (`line.as_bytes()` plus a trailing
/// `\n`), acquires a single locked stdout handle, then calls
/// [`std::io::Write::write_all`]. The lock serialises this write
/// against any other thread that also acquires the lock, so no
/// other locked writer can interleave bytes between the body and
/// the terminator. `write_all` itself does not guarantee a single
/// underlying syscall: it loops internally as the OS reports
/// partial writes, so the wire-level atomicity claim is "no other
/// LOCKED writer interleaves with us", not "exactly one write(2)
/// hits the FD".
///
/// Returns an `anyhow::Result` rather than the raw `io::Result` so
/// callers can attach context (`.context("writing notify response")`)
/// at the layer transition.
pub(crate) fn write_stdout_line(line: &str) -> Result<()> {
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    let stdout = io::stdout();
    let mut guard = stdout.lock();
    guard.write_all(&buf).context("write to stdout")?;
    Ok(())
}

/// Writes `bytes` to stdout AS-IS (no newline appended).
///
/// Used by the `aviso completions <SHELL>` subcommand, where the
/// `clap_complete::generate` callee already writes its own newlines
/// and an extra trailing one from this helper would corrupt the
/// shell script.
pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> Result<()> {
    let stdout = io::stdout();
    let mut guard = stdout.lock();
    guard.write_all(bytes).context("write to stdout")?;
    Ok(())
}

/// Returns `true` when stdout is connected to a terminal.
///
/// Wraps `std::io::IsTerminal::is_terminal`. Subcommands consult
/// this to pick the human-readable TTY form versus the NDJSON pipe
/// form per Q5; `--json` overrides this to always pick NDJSON.
pub(crate) fn is_stdout_tty() -> bool {
    io::stdout().is_terminal()
}

/// Returns whether output should be NDJSON given the resolved
/// preferences. `--json` (the `force_json` parameter) always wins;
/// otherwise NDJSON when stdout is not a TTY.
pub(crate) fn use_ndjson(force_json: bool) -> bool {
    force_json || !is_stdout_tty()
}
