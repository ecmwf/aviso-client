//! `aviso notify` subcommand.
//!
//! Pyaviso-parity publisher (per Amendment I). The single
//! positional argument is a comma-separated `key=value` list.
//! Only `event=<TYPE>` is mandatory for parameter parsing; the
//! server's notify endpoint additionally requires EVERY identifier
//! key listed in the event-type's schema (the schema's
//! `required: false` flag is a `listen`/`replay`-time filter
//! semantic, not a notify-time semantic). The `data=<JSON>` key is
//! optional; when present its value is parsed as JSON and attached
//! as the notification payload. Every other `key=value` pair
//! enters the notification identifier map.
//!
//! The parameter parser is brace-respecting: top-level commas
//! split entries, but commas inside `{}` / `[]` nesting or inside
//! `"..."` string literals are part of the value. Backslash escapes
//! inside string literals are honoured. Mismatched braces or
//! unclosed strings surface as usage errors with the offset.
//!
//! Outer `"..."` quotes around an identifier value are stripped
//! before the value is sent to the server, matching pyaviso
//! convention. This is the canonical way to pass identifier values
//! containing top-level commas (e.g.
//! `polygon="46,8,46,9,47,9,47,8,46,8"`; without the quotes the
//! commas would be parsed as parameter separators). Quote
//! stripping does NOT apply to `data=` because the JSON parser
//! handles its own quoting.

use std::collections::BTreeMap;

use anyhow::Result;
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

    let client = client_builder::build(resolved, None)?;
    let response = match client.notify(&req).await {
        Ok(r) => r,
        Err(client_err) => {
            let hint = hint_for_client_error(&client_err);
            let mut err = anyhow::Error::from(client_err).context(format!(
                "POST /api/v1/notification for event_type={event_type}"
            ));
            if let Some(h) = hint {
                err = err.context(format!("suggestion: {h}"));
            }
            return Err(err);
        }
    };

    write_response(resolved, &event_type, &response)
}

/// Inspects a `ClientError` for known patterns that point to a
/// specific operator mistake, and returns a one-line hint when a
/// match is found. Patterns chosen are high-confidence (the server's
/// own error body unambiguously names the cause); when no pattern
/// matches we return `None` and the raw `Caused by:` line is left to
/// speak for itself. The hint is appended as a `suggestion:` context
/// so `error::format_chain` renders it as a discrete line under the
/// summary.
fn hint_for_client_error(err: &aviso::ClientError) -> Option<String> {
    let aviso::ClientError::Http { status, body, .. } = err else {
        return None;
    };
    // Body-pattern hints are evaluated FIRST and are status-independent.
    // The aviso-server classifies an operator format mistake as 400 if
    // validation rejects it before processing, or 500 if processing
    // surfaces the problem (e.g. PolygonHandler parsing). Both routes
    // produce the same operator action, so we hint on the body string
    // regardless of which status the server chose.
    if body.contains("Polygon coordinates must be in pairs")
        || body.contains("Invalid latitude value")
    {
        return Some(
            "polygon values are a comma-separated list of `lat,lon` pairs (e.g. `polygon=\"46,8,46,9,47,9,47,8,46,8\"`). Wrap the value in double quotes so the top-level commas are NOT parsed as parameter separators; the CLI strips the outer quotes before sending."
                .to_string(),
        );
    }
    if body.contains("missing for notify operation") {
        return Some(
            "the schema's `required: false` flag applies to listen/replay-time filtering only; for notify, every identifier listed in the schema is required. Run `aviso schema get <TYPE>` for the full identifier set."
                .to_string(),
        );
    }
    // Status-class hints handle auth failures, where the body may be
    // HTML / empty / vendor-specific, so we rely on the HTTP semantics.
    match *status {
        401 => Some(
            "credentials are missing, invalid, or expired. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars; verify auth wired up via `aviso config dump --redact` (provider should show `<set; redacted>`)."
                .to_string(),
        ),
        403 => Some(
            "credentials were accepted but may not have notify permission for this event_type. Contact the server admin; verify the event_type with `aviso schema list`."
                .to_string(),
        ),
        _ => None,
    }
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
            "notification accepted: event_type={event_type}, status={status}, request_id={rid}, processed_at={ts}",
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

/// Strips a matched pair of outer `"..."` from an identifier
/// value. Pyaviso convention: operators wrap values containing
/// top-level commas (polygons, lists of identifiers) in double
/// quotes so the parameter splitter's brace/string awareness
/// protects the commas; the server then receives the value
/// without the wrapping quotes.
///
/// This intentionally does NOT touch the `data=` JSON payload
/// (which has its own JSON-syntactic quoting) and does NOT
/// touch single-character `"` values, which would represent an
/// unclosed-quote that the splitter would have already rejected.
fn strip_outer_quotes(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
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
                if event_type.is_some() {
                    return Err(usage_error(
                        "parameter parse: duplicate `event=` key. Each notify accepts exactly one event_type.",
                    ));
                }
                if value.is_empty() {
                    return Err(usage_error(
                        "parameter parse: `event=` requires a non-empty value",
                    ));
                }
                event_type = Some(value.clone());
            }
            "data" => {
                if payload.is_some() {
                    return Err(usage_error(
                        "parameter parse: duplicate `data=` key. Each notify accepts at most one payload; combine multiple values into a single JSON object or array.",
                    ));
                }
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
                if identifier.contains_key(key) {
                    return Err(usage_error(format!(
                        "parameter parse: duplicate identifier key `{key}`. Each identifier may appear at most once."
                    )));
                }
                identifier.insert(key.clone(), strip_outer_quotes(value));
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

    #[test]
    fn quoted_identifier_value_strips_outer_quotes_to_protect_top_level_commas() {
        let entries = split_parameters(r#"event=test_polygon,polygon="46,8,46,9,47,9""#).unwrap();
        let (event, ident, _) = build_request_parts(&entries).unwrap();
        assert_eq!(event, "test_polygon");
        assert_eq!(
            ident.get("polygon").map(String::as_str),
            Some("46,8,46,9,47,9"),
            "polygon value should be the unquoted content; got: {ident:?}"
        );
    }

    #[test]
    fn unquoted_identifier_value_passes_through_unchanged() {
        let entries = split_parameters("event=mars,class=od").unwrap();
        let (_, ident, _) = build_request_parts(&entries).unwrap();
        assert_eq!(ident.get("class").map(String::as_str), Some("od"));
    }

    #[test]
    fn data_payload_does_not_lose_outer_quote_semantics() {
        let entries = split_parameters(r#"event=mars,data="hello""#).unwrap();
        let (_, _, payload) = build_request_parts(&entries).unwrap();
        assert_eq!(
            payload.unwrap(),
            serde_json::Value::String("hello".into()),
            "data= goes through JSON parsing, not strip_outer_quotes; \"hello\" must parse as a JSON string"
        );
    }

    #[test]
    fn strip_outer_quotes_handles_edge_cases() {
        assert_eq!(strip_outer_quotes("\"abc\""), "abc");
        assert_eq!(strip_outer_quotes("abc"), "abc");
        assert_eq!(strip_outer_quotes("\""), "\"", "single char passes through");
        assert_eq!(strip_outer_quotes(""), "");
        assert_eq!(
            strip_outer_quotes("\"\""),
            "",
            "empty quoted strips to empty"
        );
        assert_eq!(
            strip_outer_quotes("a\"b"),
            "a\"b",
            "inner quotes do not strip"
        );
    }

    #[test]
    fn duplicate_event_key_rejected_with_explicit_diagnostic() {
        let entries = split_parameters("event=mars,event=dissemination").unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate `event="), "{msg}");
        assert!(msg.contains("exactly one"), "{msg}");
    }

    #[test]
    fn duplicate_data_key_rejected_with_combine_hint() {
        let entries = split_parameters(r#"event=mars,data={"a":1},data={"b":2}"#).unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate `data="), "{msg}");
        assert!(
            msg.contains("combine") || msg.contains("single JSON"),
            "{msg}"
        );
    }

    #[test]
    fn duplicate_identifier_key_rejected_naming_the_key() {
        let entries = split_parameters("event=mars,class=od,class=rd").unwrap();
        let err = build_request_parts(&entries).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("duplicate identifier"), "{msg}");
        assert!(msg.contains("class"), "{msg}");
    }

    #[test]
    fn hint_for_401_points_at_credentials_setup() {
        let err = aviso::ClientError::Http {
            status: 401,
            body: r#"{"code":"UNAUTHORIZED","message":"Invalid or expired token"}"#.to_string(),
            request_id: Some("req-xyz".into()),
        };
        let hint = hint_for_client_error(&err).expect("401 must yield a hint");
        assert!(hint.contains("credentials"), "{hint}");
        assert!(
            hint.contains("--token") || hint.contains("AVISO_TOKEN"),
            "{hint}"
        );
    }

    #[test]
    fn hint_for_403_calls_out_notify_permission_specifically() {
        let err = aviso::ClientError::Http {
            status: 403,
            body: r#"{"code":"FORBIDDEN"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("403 must yield a hint");
        assert!(hint.contains("notify permission"), "{hint}");
    }

    #[test]
    fn hint_for_400_required_field_links_listen_replay_semantics() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"message":"Required field 'date' missing for notify operation"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("400 required-missing must yield a hint");
        assert!(
            hint.contains("listen/replay") || hint.contains("filtering"),
            "hint must explain the schema's `required: false` semantic: {hint}"
        );
        assert!(hint.contains("aviso schema get"), "{hint}");
    }

    #[test]
    fn hint_for_400_polygon_format_calls_out_quoting() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"Polygon coordinates must be in pairs (lat,lon)"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("400 polygon must yield a hint");
        assert!(hint.contains("polygon"), "{hint}");
        assert!(hint.contains("double quotes"), "{hint}");
        assert!(hint.contains("lat,lon"), "{hint}");
    }

    #[test]
    fn hint_for_500_polygon_format_also_fires_status_independent() {
        let err = aviso::ClientError::Http {
            status: 500,
            body: r#"{"details":"Polygon coordinates must be in pairs (lat,lon)"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err)
            .expect("polygon hint must fire on 500 too (server may classify as 500 when caught during processing rather than validation)");
        assert!(hint.contains("polygon"), "{hint}");
        assert!(hint.contains("double quotes"), "{hint}");
    }

    #[test]
    fn hint_for_unknown_http_status_returns_none() {
        let err = aviso::ClientError::Http {
            status: 502,
            body: "<html>...</html>".to_string(),
            request_id: None,
        };
        assert!(hint_for_client_error(&err).is_none());
    }

    #[test]
    fn hint_for_non_http_client_error_returns_none() {
        let err = aviso::ClientError::Auth("test".into());
        assert!(hint_for_client_error(&err).is_none());
    }
}
