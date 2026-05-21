//! `aviso notify` subcommand.
//!
//! Pyaviso-parity publisher (per Amendment I). The single
//! positional argument is a comma-separated `key=value` list.
//! Only `event=<TYPE>` is mandatory: it names the schema-configured
//! event type. The `data=<JSON>` key is optional; when present its
//! value is parsed as JSON and attached as the notification
//! payload. Every other `key=value` pair enters the notification
//! identifier map.
//!
//! The parameter parser is brace-respecting: top-level commas
//! split entries, but commas inside `{}` / `[]` nesting or inside
//! `"..."` string literals are part of the value. Backslash escapes
//! inside string literals are honoured. Mismatched braces or
//! unclosed strings surface as usage errors with the offset.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use aviso::NotificationRequest;

use crate::client_builder;
use crate::config::Resolved;
use crate::exit::usage_error;
use crate::output;

/// Runs the `aviso notify` subcommand.
pub(crate) async fn run(resolved: &Resolved, parameters: &str) -> Result<()> {
    let entries = split_parameters(parameters)?;
    let (event_type, identifier, payload) = build_request_parts(&entries)?;

    let mut req = NotificationRequest::new(event_type.clone()).with_identifier(identifier);
    if let Some(p) = payload {
        req = req.with_payload(p);
    }

    let client = client_builder::build(resolved)?;
    let response = client
        .notify(&req)
        .await
        .with_context(|| format!("POST /api/v1/notification for event_type={event_type}"))?;

    write_response(resolved, &event_type, &response)
}

fn write_response(
    resolved: &Resolved,
    event_type: &str,
    response: &aviso::NotifyResponse,
) -> Result<()> {
    if output::use_ndjson(resolved.force_json) {
        let value = serde_json::json!({
            "event_type": event_type,
            "status": response.status,
            "request_id": response.request_id,
            "processed_at": response.processed_at,
        });
        output::write_stdout_line(&serde_json::to_string(&value)?)
    } else {
        let line = format!(
            "notification accepted: event={event_type}, status={status}, request_id={rid}, at={ts}",
            status = response.status,
            rid = response.request_id,
            ts = response.processed_at,
        );
        output::write_stdout_line(&line)
    }
}

/// Splits a comma-separated `key=value` parameter string with
/// brace and string awareness. Returns the entries in argv order.
fn split_parameters(s: &str) -> Result<Vec<(String, String)>> {
    let mut entries = Vec::new();
    let mut depth: u32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => depth = depth.saturating_add(1),
            '}' | ']' => {
                if depth == 0 {
                    return Err(usage_error(format!(
                        "parameter parse: unmatched closing brace at offset {i} in `{s}`"
                    )));
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                push_kv(&mut entries, &s[start..i])?;
                start = i + 1;
            }
            _ => {}
        }
    }
    if in_string {
        return Err(usage_error(format!(
            "parameter parse: unclosed `\"` in `{s}`"
        )));
    }
    if depth > 0 {
        return Err(usage_error(format!(
            "parameter parse: unclosed `{{` (or `[`) in `{s}`"
        )));
    }
    let tail = &s[start..];
    if !tail.is_empty() {
        push_kv(&mut entries, tail)?;
    }
    Ok(entries)
}

fn push_kv(out: &mut Vec<(String, String)>, slice: &str) -> Result<()> {
    let slice = slice.trim();
    if slice.is_empty() {
        return Ok(());
    }
    let eq = slice.find('=').ok_or_else(|| {
        usage_error(format!(
            "parameter parse: no `=` in entry `{slice}` (expected key=value)"
        ))
    })?;
    let key = slice[..eq].trim().to_string();
    if key.is_empty() {
        return Err(usage_error(format!(
            "parameter parse: empty key in entry `{slice}`"
        )));
    }
    let value = slice[eq + 1..].to_string();
    out.push((key, value));
    Ok(())
}

fn build_request_parts(
    entries: &[(String, String)],
) -> Result<(String, BTreeMap<String, String>, Option<serde_json::Value>)> {
    let mut event_type: Option<String> = None;
    let mut payload: Option<serde_json::Value> = None;
    let mut identifier: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in entries {
        match key.as_str() {
            "event" => {
                if value.is_empty() {
                    return Err(usage_error(
                        "parameter parse: `event=` requires a non-empty value",
                    ));
                }
                event_type = Some(value.clone());
            }
            "data" => {
                if value.is_empty() {
                    return Err(usage_error(
                        "parameter parse: `data=` is empty; use `data=\"\"` for an empty JSON string or `data=null` for an explicit null",
                    ));
                }
                let parsed: serde_json::Value = serde_json::from_str(value).map_err(|e| {
                    usage_error(format!(
                        "parameter parse: `data=` is not valid JSON at line {l} column {c}: {msg}",
                        l = e.line(),
                        c = e.column(),
                        msg = e
                    ))
                })?;
                payload = Some(parsed);
            }
            _ => {
                identifier.insert(key.clone(), value.clone());
            }
        }
    }
    let event_type =
        event_type.ok_or_else(|| usage_error("parameter parse: `event=<TYPE>` is required"))?;
    Ok((event_type, identifier, payload))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on parsing success is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn parts(input: &str) -> (String, BTreeMap<String, String>, Option<serde_json::Value>) {
        let entries = split_parameters(input).unwrap();
        build_request_parts(&entries).unwrap()
    }

    #[test]
    fn simple_event_and_identifiers() {
        let (event, ident, payload) = parts("event=mars,class=od,stream=oper");
        assert_eq!(event, "mars");
        assert_eq!(ident.get("class").map(String::as_str), Some("od"));
        assert_eq!(ident.get("stream").map(String::as_str), Some("oper"));
        assert!(payload.is_none());
    }

    #[test]
    fn embedded_json_object_payload() {
        let (event, ident, payload) = parts("event=mars,data={\"a\":1,\"b\":2}");
        assert_eq!(event, "mars");
        assert!(ident.is_empty());
        let payload = payload.unwrap();
        assert_eq!(payload["a"], 1);
        assert_eq!(payload["b"], 2);
    }

    #[test]
    fn embedded_json_array_payload() {
        let (event, _, payload) = parts("event=mars,data=[1,2,3]");
        assert_eq!(event, "mars");
        let payload = payload.unwrap();
        assert!(payload.is_array());
        assert_eq!(payload.as_array().unwrap().len(), 3);
    }

    #[test]
    fn quoted_string_with_literal_comma() {
        let (_, _, payload) = parts(r#"event=mars,data={"msg":"hello, world"}"#);
        let payload = payload.unwrap();
        assert_eq!(payload["msg"], "hello, world");
    }

    #[test]
    fn escaped_quote_inside_string() {
        let (_, _, payload) = parts(r#"event=mars,data={"msg":"she said \"hi\""}"#);
        let payload = payload.unwrap();
        assert_eq!(payload["msg"], "she said \"hi\"");
    }

    #[test]
    fn missing_event_key_errors() {
        let entries = split_parameters("class=od,stream=oper").unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("event="), "{msg}");
    }

    #[test]
    fn empty_event_value_errors() {
        let entries = split_parameters("event=").unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("event="), "{msg}");
    }

    #[test]
    fn empty_data_value_errors_with_helpful_suggestion() {
        let entries = split_parameters("event=mars,data=").unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("data="), "{msg}");
        assert!(
            msg.contains("data=\"\"") || msg.contains("empty JSON"),
            "{msg}"
        );
    }

    #[test]
    fn unclosed_brace_errors() {
        let err = split_parameters("event=mars,data={bad").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unclosed"), "{msg}");
    }

    #[test]
    fn unmatched_closing_brace_errors() {
        let err = split_parameters("event=mars,data=}").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unmatched"), "{msg}");
    }

    #[test]
    fn unclosed_quoted_string_errors() {
        let err = split_parameters("event=mars,data=\"oops").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unclosed"), "{msg}");
    }

    #[test]
    fn invalid_json_in_data_errors_with_line_column() {
        let entries = split_parameters(r#"event=mars,data={"a":}"#).unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("data="), "{msg}");
        assert!(msg.contains("line"), "{msg}");
    }

    #[test]
    fn empty_entry_between_commas_is_skipped() {
        let entries = split_parameters("event=mars,,class=od").unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn entry_without_equals_errors() {
        let err = split_parameters("event=mars,nokey").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains('='), "{msg}");
    }
}
