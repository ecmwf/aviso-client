//! Echo trigger dispatch.

use crate::Notification;

use super::TriggerError;

/// Echo dispatch: serialise the notification into a buffer ONCE (appending
/// the newline to the same buffer), then a single `write_all` against a
/// locked stdout handle. Buffer-then-write avoids any intra-trigger seam
/// between the JSON body and the line terminator.
///
/// TTY-aware output: when stdout is connected to a terminal, the
/// notification is rendered as a compact human-readable line
/// (`event_type#sequence  k=v k=v  =>  {payload}`) suitable for scanning
/// during interactive listener sessions. When stdout is NOT a terminal
/// (piped, redirected to file, captured by a downstream tool), the
/// original single-line JSON shape is emitted so machine consumers
/// (`aviso listen | jq`, file ingestion pipelines) continue to parse
/// it unchanged.
pub(super) fn dispatch_echo(notification: &Notification) -> Result<(), TriggerError> {
    use std::io::IsTerminal as _;
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let is_tty = stdout.is_terminal();
    // Color choice is process-wide and set by the CLI before the
    // first dispatch; `crate::echo_color_enabled()` reads the
    // `AtomicBool` set by `crate::set_echo_color_enabled(bool)`.
    // Library consumers that do NOT call the setter get the
    // default `false` (no color). Detection precedence (TTY +
    // NO_COLOR) lives in the CLI, not here, so the lib stays a
    // pure mechanism with policy delegated to the caller.
    let use_color = crate::echo_color_enabled();
    let mut buf: Vec<u8> = if is_tty {
        format_human(notification, use_color)?.into_bytes()
    } else {
        serde_json::to_vec(notification)?
    };
    // `format_human` already terminates with `\n`; the extra blank
    // line below separates consecutive multi-line notification blocks
    // visually. For the JSON branch, the single `\n` terminates one
    // NDJSON record.
    buf.push(b'\n');
    let mut handle = stdout.lock();
    handle.write_all(&buf)?;
    Ok(())
}

/// Renders a [`Notification`] as a multi-line, kubectl-describe-style
/// block intended for human reading. Returns a [`String`] terminated
/// by a final `\n`; the caller (`dispatch_echo`) appends another `\n`
/// so consecutive notifications are visually separated by a blank
/// line.
///
/// Output shape:
/// ```text
/// mars #15
///   class:    od
///   date:     20260521
///   domain:   g
///   expver:   0001
///   step:     0
///   stream:   oper
///   time:     1200
///   payload:  {"region":"north"}
/// ```
///
/// Identifier keys are right-padded so the values column aligns, the
/// payload (when present) appears as the last field with the same
/// alignment, and the heading `<event_type> #<sequence>` is unindented
/// so the eye finds the next event at a glance.
///
/// When `use_color` is `true`, ANSI escape codes wrap the heading in
/// cyan and the field labels in dim. Color is opt-in only via the CLI
/// `--color auto|always` flag; the plain-text branch is the canonical
/// form for tests and machine pipelines.
pub(super) fn format_human(n: &Notification, use_color: bool) -> Result<String, TriggerError> {
    use std::fmt::Write as _;
    let (cyan, dim, reset) = if use_color {
        ("\x1b[36m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    let payload_present = !n.payload.is_null();
    let key_width = n
        .identifier
        .keys()
        .map(String::len)
        .chain(payload_present.then_some("payload".len()))
        .max()
        .unwrap_or(0);

    let mut s = format!("{cyan}{} #{}{reset}\n", n.event_type, n.sequence);
    for (k, v) in &n.identifier {
        let _ = writeln!(s, "  {dim}{k:key_width$}{reset}  {v}");
    }
    if payload_present {
        let payload_json = serde_json::to_string(&n.payload)?;
        let _ = writeln!(
            s,
            "  {dim}{:key_width$}{reset}  {}",
            "payload", payload_json
        );
    }
    Ok(s)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on format_human (which only fails on serde_json infallible-for-Value serialisation) is the expected diagnostic"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::{dispatch_echo, format_human};
    use crate::Notification;

    fn make_notification() -> Notification {
        Notification {
            event_type: "mars".to_string(),
            sequence: 1,
            identifier: BTreeMap::new(),
            payload: serde_json::Value::Null,
        }
    }

    #[test]
    fn echo_trigger_succeeds_without_retry() {
        let result = dispatch_echo(&make_notification());
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn format_human_heading_only_when_no_identifiers_or_payload() {
        let s = format_human(&make_notification(), false).unwrap();
        assert_eq!(s, "mars #1\n");
    }

    #[test]
    fn format_human_aligned_identifier_block() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        identifier.insert("date".to_string(), "20260521".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 42,
            identifier,
            payload: serde_json::Value::Null,
        };
        let s = format_human(&n, false).unwrap();
        let expected = "mars #42\n  class  od\n  date   20260521\n";
        assert_eq!(s, expected, "actual:\n{s}\nexpected:\n{expected}");
    }

    #[test]
    fn format_human_payload_appears_as_last_field_with_same_alignment() {
        let n = Notification {
            event_type: "test_polygon".to_string(),
            sequence: 7,
            identifier: BTreeMap::new(),
            payload: serde_json::json!({"region": "north"}),
        };
        let s = format_human(&n, false).unwrap();
        let expected = "test_polygon #7\n  payload  {\"region\":\"north\"}\n";
        assert_eq!(s, expected, "actual:\n{s}\nexpected:\n{expected}");
    }

    #[test]
    fn format_human_full_block_with_identifiers_and_payload() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        identifier.insert("date".to_string(), "20260521".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 12,
            identifier,
            payload: serde_json::json!({"test": true}),
        };
        let s = format_human(&n, false).unwrap();
        let expected = "mars #12\n  class    od\n  date     20260521\n  payload  {\"test\":true}\n";
        assert_eq!(s, expected, "actual:\n{s}\nexpected:\n{expected}");
    }

    #[test]
    fn format_human_omits_payload_line_when_payload_is_json_null() {
        let s = format_human(&make_notification(), false).unwrap();
        assert!(
            !s.contains("payload"),
            "JSON null payload must not produce a `payload` line in human output: {s}"
        );
    }

    #[test]
    fn format_human_with_color_wraps_heading_in_cyan() {
        let s = format_human(&make_notification(), true).unwrap();
        assert!(
            s.starts_with("\x1b[36mmars #1\x1b[0m"),
            "heading must be cyan-wrapped: {s:?}"
        );
    }

    #[test]
    fn format_human_with_color_dims_identifier_labels() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 12,
            identifier,
            payload: serde_json::Value::Null,
        };
        let s = format_human(&n, true).unwrap();
        assert!(
            s.contains("\x1b[2mclass\x1b[0m"),
            "identifier label `class` must be dim-wrapped: {s:?}"
        );
    }

    #[test]
    fn format_human_without_color_emits_no_ansi_escapes() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 12,
            identifier,
            payload: serde_json::json!({"test": true}),
        };
        let s = format_human(&n, false).unwrap();
        assert!(
            !s.contains('\x1b'),
            "plain output must not contain any ANSI escape: {s:?}"
        );
    }
}
