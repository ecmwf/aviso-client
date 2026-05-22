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
    let mut buf: Vec<u8> = if stdout.is_terminal() {
        format_human(notification)?.into_bytes()
    } else {
        serde_json::to_vec(notification)?
    };
    buf.push(b'\n');
    let mut handle = stdout.lock();
    handle.write_all(&buf)?;
    Ok(())
}

/// Renders a [`Notification`] as one human-readable line.
///
/// Shape:
/// - `<event_type>#<sequence>` always.
/// - identifier `k=v` pairs separated by spaces, when non-empty.
/// - `  =>  <payload-json>` when payload is not JSON null.
///
/// Identifier values are written verbatim; values containing spaces or
/// `=` are unusual in aviso schemas in practice and any operator who
/// hits one can fall back to piping the listener through `jq` and
/// reading the JSON form.
pub(super) fn format_human(n: &Notification) -> Result<String, TriggerError> {
    use std::fmt::Write as _;
    let mut s = format!("{}#{}", n.event_type, n.sequence);
    if !n.identifier.is_empty() {
        s.push(' ');
        for (k, v) in &n.identifier {
            let _ = write!(s, " {k}={v}");
        }
    }
    if !n.payload.is_null() {
        s.push_str("  =>  ");
        s.push_str(&serde_json::to_string(&n.payload)?);
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
    fn format_human_event_type_and_sequence_always_present() {
        let line = format_human(&make_notification()).unwrap();
        assert_eq!(line, "mars#1");
    }

    #[test]
    fn format_human_includes_identifiers_when_non_empty() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        identifier.insert("date".to_string(), "20260521".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 42,
            identifier,
            payload: serde_json::Value::Null,
        };
        let line = format_human(&n).unwrap();
        assert_eq!(line, "mars#42  class=od date=20260521");
    }

    #[test]
    fn format_human_includes_payload_when_non_null() {
        let n = Notification {
            event_type: "test_polygon".to_string(),
            sequence: 7,
            identifier: BTreeMap::new(),
            payload: serde_json::json!({"region": "north"}),
        };
        let line = format_human(&n).unwrap();
        assert_eq!(line, r#"test_polygon#7  =>  {"region":"north"}"#);
    }

    #[test]
    fn format_human_with_identifiers_and_payload() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        let n = Notification {
            event_type: "mars".to_string(),
            sequence: 12,
            identifier,
            payload: serde_json::json!({"test": true}),
        };
        let line = format_human(&n).unwrap();
        assert_eq!(line, r#"mars#12  class=od  =>  {"test":true}"#);
    }

    #[test]
    fn format_human_omits_payload_arrow_when_payload_is_json_null() {
        let line = format_human(&make_notification()).unwrap();
        assert!(
            !line.contains("=>"),
            "JSON null payload must not produce a `=>` arrow in human output: {line}"
        );
    }
}
