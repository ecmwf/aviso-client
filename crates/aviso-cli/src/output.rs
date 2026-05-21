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
//! TTY detection via `std::io::IsTerminal` will land alongside the
//! handlers that need it (notify, schema list) in a follow-up
//! source change.

use std::io::{self, Write};

use anyhow::{Context, Result};

/// Writes `line` to stdout followed by a newline, atomically.
///
/// The buffer-then-write shape (`Vec<u8>` carrying `line.as_bytes()`
/// plus a trailing `\n`, then one `write_all` against a locked
/// stdout handle) matches the Echo trigger's atomicity contract:
/// the entire line lands in a single syscall on the happy path, so
/// a broken pipe cannot leave a half-written line between the body
/// and the terminator.
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


