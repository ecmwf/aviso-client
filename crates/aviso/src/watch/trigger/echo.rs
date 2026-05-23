//! Echo trigger dispatch.

use crate::Notification;

use super::TriggerError;

/// Echo dispatch: serialise the notification into a buffer ONCE (appending
/// the newline to the same buffer), then a single `write_all` against a
/// locked stdout handle. Buffer-then-write avoids any intra-trigger seam
/// between the JSON body and the line terminator.
///
/// TTY-aware output: when stdout is connected to a terminal, the
/// notification is rendered as a one-line dim leader
/// (`new notification (trigger: echo):`) followed by the pretty-printed
/// JSON object representing the whole `Notification`. When stdout is NOT
/// a terminal (piped, redirected to file, captured by a downstream tool),
/// the single-line NDJSON shape is emitted so machine consumers
/// (`aviso listen | jq`, file ingestion pipelines) continue to parse it
/// unchanged.
pub(super) fn dispatch_echo(
    notification: &Notification,
    listener_label: Option<&str>,
) -> Result<(), TriggerError> {
    use std::io::IsTerminal as _;
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let is_tty = stdout.is_terminal();
    let use_color = crate::echo_color_enabled();
    let mut buf: Vec<u8> = if is_tty {
        format_human(notification, use_color, listener_label)?.into_bytes()
    } else {
        serde_json::to_vec(notification)?
    };
    buf.push(b'\n');
    let mut handle = stdout.lock();
    handle.write_all(&buf)?;
    Ok(())
}

/// Leader emitted above the pretty-printed JSON body when no listener
/// label is attached; the labelled form is built by `leader_text`.
const TTY_LEADER: &str = "new notification (trigger: echo):";

fn leader_text(label: Option<&str>) -> String {
    match label {
        Some(name) => format!("new notification (listener: {name}, trigger: echo):"),
        None => TTY_LEADER.to_string(),
    }
}

/// Renders a [`Notification`] as a one-line dim leader followed by a
/// multi-line pretty-printed JSON object, intended for human reading
/// during an interactive `aviso listen` session.
///
/// Output shape (plain text, no color):
/// ```text
/// new notification (trigger: echo):
/// {
///   "event_type": "test_polygon",
///   "sequence": 33,
///   "identifier": {
///     "date": "20260522",
///     "time": "1200"
///   },
///   "payload": {
///     "note": "should be picked up",
///     "test": true
///   }
/// }
/// ```
///
/// The returned string terminates with a single `\n`; `dispatch_echo`
/// appends another `\n` so consecutive notifications are visually
/// separated by a blank line.
///
/// When `use_color` is `true`, the leader line is wrapped in ANSI dim
/// escapes (the JSON body stays plain so it remains copy-paste-friendly
/// into `jq` and other tools). Color is opt-in only via the CLI
/// `--color auto|always` flag; the plain-text branch is the canonical
/// form for tests and machine pipelines.
pub(super) fn format_human(
    n: &Notification,
    use_color: bool,
    listener_label: Option<&str>,
) -> Result<String, TriggerError> {
    let (dim, reset) = if use_color {
        ("\x1b[2m", "\x1b[0m")
    } else {
        ("", "")
    };

    let body = serde_json::to_string_pretty(n)?;
    let leader = leader_text(listener_label);
    let mut s = String::with_capacity(leader.len() + body.len() + 32);
    s.push_str(dim);
    s.push_str(&leader);
    s.push_str(reset);
    s.push('\n');
    s.push_str(&body);
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on format_human (which only fails on serde_json infallible-for-Value serialisation) and on string-slice helpers is the expected diagnostic"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::{TTY_LEADER, dispatch_echo, format_human};
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
        let result = dispatch_echo(&make_notification(), None);
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn format_human_starts_with_leader_line_then_pretty_json_body() {
        let s = format_human(&make_notification(), false, None).unwrap();
        let first_line = s.lines().next().expect("output has at least one line");
        assert_eq!(
            first_line, TTY_LEADER,
            "first line must be the dim leader announcing the notification + trigger; got {first_line:?}"
        );
        let body = s.strip_prefix(TTY_LEADER).and_then(|r| r.strip_prefix('\n')).expect(
            "after the leader line and its newline, the rest must be the JSON body terminated by a final newline",
        );
        let body = body
            .strip_suffix('\n')
            .expect("format_human output must end with a single trailing newline");
        let parsed: serde_json::Value = serde_json::from_str(body).expect(
            "the body MUST parse as a JSON object (this is the contract jq-piping consumers rely on for the pipe-mode form; the TTY form is the same shape with whitespace)",
        );
        assert_eq!(parsed["event_type"], "mars");
        assert_eq!(parsed["sequence"], 1);
    }

    #[test]
    fn format_human_body_is_serde_json_to_string_pretty_of_the_whole_notification() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        identifier.insert("date".to_string(), "20260521".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 12,
            identifier,
            payload: serde_json::json!({"test": true}),
        };
        let s = format_human(&n, false, None).unwrap();
        let expected_body = serde_json::to_string_pretty(&n).unwrap();
        let expected = format!("{TTY_LEADER}\n{expected_body}\n");
        assert_eq!(
            s, expected,
            "TTY format must be the leader line then the SAME pretty JSON serde_json::to_string_pretty would produce; got:\n{s}\n---\nexpected:\n{expected}",
        );
    }

    #[test]
    fn format_human_includes_identifier_keys_and_payload_in_pretty_json_block() {
        let mut identifier = BTreeMap::new();
        identifier.insert("date".to_string(), "20260522".to_string());
        identifier.insert("time".to_string(), "1200".to_string());
        let n = Notification {
            event_type: "test_polygon".to_string(),
            sequence: 33,
            identifier,
            payload: serde_json::json!({"note": "should be picked up", "test": true}),
        };
        let s = format_human(&n, false, None).unwrap();
        for expected_fragment in &[
            "\"event_type\": \"test_polygon\"",
            "\"sequence\": 33",
            "\"identifier\": {",
            "\"date\": \"20260522\"",
            "\"time\": \"1200\"",
            "\"payload\": {",
            "\"note\": \"should be picked up\"",
            "\"test\": true",
        ] {
            assert!(
                s.contains(expected_fragment),
                "pretty JSON body must contain {expected_fragment:?}; full output:\n{s}",
            );
        }
    }

    #[test]
    fn format_human_with_null_payload_still_includes_payload_field_as_json_null() {
        let s = format_human(&make_notification(), false, None).unwrap();
        assert!(
            s.contains("\"payload\": null"),
            "the whole-Notification JSON form must emit payload as JSON null (NOT silently drop the field as the previous kubectl-describe format did); without this, a downstream consumer that copy-pastes the TTY body into jq would see a different shape than what the pipe form produces. Got:\n{s}",
        );
    }

    #[test]
    fn format_human_with_color_wraps_leader_in_dim_and_leaves_json_body_uncoloured() {
        let s = format_human(&make_notification(), true, None).unwrap();
        let expected_leader_line = format!("\x1b[2m{TTY_LEADER}\x1b[0m");
        let first_line = s.lines().next().unwrap();
        assert_eq!(
            first_line, expected_leader_line,
            "leader line must be dim-wrapped; got {first_line:?}",
        );
        let body = s
            .strip_prefix(&expected_leader_line)
            .and_then(|r| r.strip_prefix('\n'))
            .unwrap();
        assert!(
            !body.contains('\x1b'),
            "JSON body must remain plain (no ANSI escapes) so it stays copy-paste-friendly into `jq` and similar tools even when --color always is set: {body:?}",
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
        let s = format_human(&n, false, None).unwrap();
        assert!(
            !s.contains('\x1b'),
            "plain output must not contain any ANSI escape: {s:?}",
        );
    }

    #[test]
    fn format_human_with_listener_label_names_the_listener_inline_in_leader() {
        let s = format_human(&make_notification(), false, Some("mars-od")).unwrap();
        let first_line = s.lines().next().unwrap();
        assert_eq!(
            first_line, "new notification (listener: mars-od, trigger: echo):",
            "labelled leader must inline the listener name in the parenthetical so multi-listener configs disambiguate; got {first_line:?}",
        );
    }

    #[test]
    fn format_human_with_label_and_color_dims_the_labelled_leader_line() {
        let s = format_human(&make_notification(), true, Some("alpha")).unwrap();
        let first_line = s.lines().next().unwrap();
        assert_eq!(
            first_line, "\x1b[2mnew notification (listener: alpha, trigger: echo):\x1b[0m",
            "the entire labelled leader (including the parenthetical) must be wrapped in dim ANSI escapes when color is on; got {first_line:?}",
        );
    }

    #[test]
    fn format_human_terminates_with_a_single_newline_so_dispatch_echo_blank_line_separator_works() {
        let s = format_human(&make_notification(), false, None).unwrap();
        assert!(
            s.ends_with('\n') && !s.ends_with("\n\n"),
            "format_human must end with EXACTLY one '\\n'; dispatch_echo appends a second '\\n' to produce the blank-line separator between consecutive notifications, so a trailing double-newline here would produce TRIPLE newlines in the rendered stream. Got tail: {:?}",
            s.chars().rev().take(4).collect::<String>(),
        );
    }
}
