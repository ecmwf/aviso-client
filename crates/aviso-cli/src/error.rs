//! Top-level Error UX formatter for the `aviso` binary.
//!
//! Every CLI subcommand returns `anyhow::Result<()>`. The binary's
//! `main` routes every `Err(_)` through [`format_chain`] which
//! prints to stderr in a discipline that matches the nine-point
//! Error UX rule from the locked design:
//!
//! 1. exits with the right code (0 success, 1 runtime, 2 usage,
//!    130 hard-SIGINT); the exit code is the caller's job, this
//!    module just formats.
//! 2. one-line summary first.
//! 3. absolute path when a file was involved (the caller attaches
//!    paths via `.with_context(|| format!("at: {}", path.display()))`).
//! 4. YAML line and column reported via
//!    `serde_norway::Error::location()` when present, surfaced
//!    inside the anyhow Caused-by chain.
//! 5. key path NAMED for serde errors via the `serde_norway`
//!    Display impl (which embeds the path).
//! 6. suggestion attached as a top-level chain context with the
//!    `suggestion:` prefix; the formatter recognises that prefix
//!    and reformats accordingly.
//! 7. anyhow "Caused by:" chain printed unconditionally.
//! 8. never leaks paths the user did not provide, env values, or
//!    secrets; the lib's redaction-at-Display discipline (trigger
//!    surface, Bearer/Basic Debug) handles this at the source.
//! 9. every subcommand routes through this single seam.

use std::io::{self, Write};

use anyhow::Error;

/// Formats an anyhow error chain to stderr in the project's
/// Error UX layout.
///
/// The format is:
///
/// ```text
/// error: <top-level message>
/// Caused by:
///   0: <next-level message>
///   1: <deeper-level message>
///   ...
/// ```
///
/// A top-level message that starts with `"suggestion: "` is reformatted
/// onto its own line WITHOUT the chain prefix so the user sees
/// the suggestion as a discrete action rather than a nested cause.
/// Any context entry whose message begins with `"at: "` is treated
/// similarly: rendered as a leading `at: <PATH>` line under the
/// summary.
///
/// [`crate::exit::UsageErrorTag`] is filtered out of the displayed
/// chain. The tag is an exit-code marker only (it routes the process
/// to exit `2` via [`crate::exit::exit_code_for_anyhow`]); it carries
/// no actionable detail for the user and rendering its Display
/// (`"usage error"`) as a trailing `Caused by:` entry adds noise
/// without information. The tag still lives in the underlying
/// `anyhow::Error` so the exit-code routing keeps working unchanged.
pub(crate) fn format_chain(err: &Error) {
    let mut stderr = io::stderr().lock();

    let entries: Vec<String> = err
        .chain()
        .filter(|e| !e.is::<crate::exit::UsageErrorTag>())
        .map(ToString::to_string)
        .collect();
    let mut summary: Option<&str> = None;
    let mut prefix_lines: Vec<String> = Vec::new();
    let mut caused_by: Vec<&str> = Vec::new();

    for entry in &entries {
        if let Some(stripped) = entry.strip_prefix("suggestion: ") {
            prefix_lines.push(format!("suggestion: {stripped}"));
            continue;
        }
        if let Some(stripped) = entry.strip_prefix("at: ") {
            prefix_lines.push(format!("at: {stripped}"));
            continue;
        }
        if summary.is_none() {
            summary = Some(entry.as_str());
        } else {
            caused_by.push(entry.as_str());
        }
    }

    let summary = summary.unwrap_or("operation failed");
    let _ = writeln!(stderr, "error: {summary}");
    for line in &prefix_lines {
        let _ = writeln!(stderr, "{line}");
    }
    if !caused_by.is_empty() {
        let _ = writeln!(stderr, "Caused by:");
        for (idx, msg) in caused_by.iter().enumerate() {
            let _ = writeln!(stderr, "  {idx}: {msg}");
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic chains is the expected diagnostic"
)]
mod tests {
    use super::*;

    #[test]
    fn chain_walks_top_to_bottom() {
        let err: Error = anyhow::anyhow!("missing field `type` at line 7 column 3")
            .context("parse config file")
            .context("loading listener");
        let mut count = 0;
        for c in err.chain() {
            count += 1;
            assert!(!c.to_string().is_empty(), "chain entry {count}");
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn suggestion_prefix_recognised_for_special_rendering() {
        let err: Error = anyhow::anyhow!("aviso admin wipe-all requires explicit confirmation")
            .context("suggestion: add --yes (this command is destructive and not reversible)");
        let chain: Vec<String> = err.chain().map(ToString::to_string).collect();
        assert!(chain.iter().any(|s| s.starts_with("suggestion: ")));
    }

    #[test]
    fn at_prefix_recognised_for_special_rendering() {
        let err: Error = anyhow::anyhow!("YAML parse error: missing field `type`")
            .context("at: /home/alice/.config/aviso/config.yaml");
        let chain: Vec<String> = err.chain().map(ToString::to_string).collect();
        assert!(chain.iter().any(|s| s.starts_with("at: ")));
    }

    #[test]
    fn usage_error_tag_is_filtered_from_displayed_chain() {
        let err = crate::exit::usage_error("missing --yes for destructive admin command")
            .context("running `aviso admin wipe-all`");
        let displayed: Vec<String> = err
            .chain()
            .filter(|e| !e.is::<crate::exit::UsageErrorTag>())
            .map(ToString::to_string)
            .collect();
        assert!(
            !displayed.iter().any(|s| s == "usage error"),
            "the `usage error` tag string MUST NOT appear in the user-visible chain (it is an internal exit-code marker, not an actionable cause); got: {displayed:?}",
        );
        assert!(
            displayed.iter().any(|s| s.contains("missing --yes")),
            "filter must keep the actual user-facing messages: {displayed:?}",
        );
    }

    #[test]
    fn usage_error_tag_filter_does_not_break_exit_code_routing() {
        let err = crate::exit::usage_error("missing --yes for destructive admin command")
            .context("running `aviso admin wipe-all`");
        assert_eq!(
            crate::exit::exit_code_for_anyhow(&err),
            crate::exit::USAGE_ERROR,
            "filtering the tag from the DISPLAYED chain must not remove it from the underlying anyhow::Error; exit-code routing relies on downcast and must still see the tag",
        );
    }
}
