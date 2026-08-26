// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `aviso notify` subcommand.
//!
//! Pyaviso-parity publisher (per Amendment I). The single
//! positional argument is a comma-separated parameter list.
//! Only `event=<TYPE>` is mandatory for parameter parsing; the
//! server's notify endpoint additionally requires EVERY identifier
//! key listed in the event-type's schema (the schema's
//! `required: false` flag is a `listen`/`replay`-time filter
//! semantic, not a notify-time semantic). The `data=<JSON>` key is
//! optional; when present its value is parsed as JSON and attached
//! as the notification payload. Every other `key=value` pair enters
//! the notification identifier map. `key:=JSON` explicitly parses any JSON
//! value. The compatible `key=value` form parses values beginning with `[` or
//! `{` as structured JSON and keeps all other values as strings.
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

    let client = client_builder::build(resolved, None, false)?;
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
    if body.contains("UNKNOWN_EVENT_TYPE") || body.contains("unknown event type") {
        return Some(
            "the event_type is not configured on the server. The response above includes a `configured_event_types` array listing every event_type the server accepts; run `aviso schema list` for the same list. Check for a typo in `event=<TYPE>`."
                .to_string(),
        );
    }
    if let Some(hint) = polygon_violation_hint(body, "notify") {
        return Some(hint);
    }
    if body.contains("missing for notify operation") {
        return Some(
            "the schema's `required: false` flag applies to listen/replay-time filtering only; for notify, every identifier listed in the schema is required. Run `aviso schema get <TYPE>` for the full identifier set."
                .to_string(),
        );
    }
    if let Some(hint) = constraint_violation_hint(body, "notify") {
        return Some(hint);
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

/// Per-sub-message polygon validation hint. Hoisted out of the
/// per-subcommand dispatchers so notify, listen, AND replay share
/// the SAME polygon advice for the same six canonical server
/// error variants (odd count, non-numeric coord, < 4 pairs, empty,
/// out-of-range lat/lon, fallback). The subcommand-specific closing
/// suffix (CLI quoting advice vs YAML syntax vs replay --identifiers
/// JSON) is keyed on `subcommand` so the hint reads naturally in
/// each caller without diverging on the core diagnosis.
pub(crate) fn polygon_violation_hint(body: &str, subcommand: &str) -> Option<String> {
    if !body.contains("must be a valid polygon") {
        return None;
    }
    let specific = if body.contains("odd number of values") {
        "coordinates must come in lat,lon pairs (even total count)"
    } else if body.contains("could not parse latitude")
        || body.contains("could not parse longitude")
    {
        "each coordinate must be a number; check for typos and confirm you're using comma (not semicolon) as the delimiter"
    } else if body.contains("at least 4 coordinate pairs") {
        "a polygon needs at least 4 coordinate pairs: 3 unique vertices plus a closing repeat of the first vertex"
    } else if body.contains("polygon coordinate string is empty") {
        "polygon value cannot be empty"
    } else if body.contains("outside the valid range") {
        "latitude must be in [-90, 90] and longitude in [-180, 180] (note the order: each pair is `lat,lon`, not `lon,lat`)"
    } else {
        "polygon must be a comma-separated list of lat,lon pairs"
    };
    let suffix = match subcommand {
        "listen" | "watch" => {
            "For inline listen, set polygon inside the `--identifiers` JSON object (e.g. `--identifiers '{\"polygon\":\"46,8,46,9,47,9,47,8,46,8\"}'`); for YAML-driven listen, set `polygon: \"46,8,46,9,47,9,47,8,46,8\"` in the listener YAML (the value wrapped as a single string)."
        }
        "replay" => {
            "For ad-hoc replay, set polygon inside the `--identifiers` JSON object (e.g. `--identifiers '{\"polygon\":\"46,8,46,9,47,9,47,8,46,8\"}'`); for YAML-driven replay, use the same `polygon:` shape as listener YAML."
        }
        _ => {
            "Example: `polygon=\"46,8,46,9,47,9,47,8,46,8\"` (4 vertices, closed polygon; the surrounding double quotes prevent the inner commas from being parsed as parameter separators)."
        }
    };
    Some(format!("{specific}. {suffix}"))
}

/// Per-handler-type validation hint for the server-side schema
/// constraint errors that aviso-server emits (StringHandler max_length,
/// EnumHandler allowed values, IntHandler range, DateHandler format,
/// TimeHandler format).
///
/// The hint repeats the constraint as a self-contained tip AND always
/// suggests `aviso schema get <TYPE>` for the authoritative shape.
/// The `subcommand` parameter scopes the closing suggestion to the
/// caller's idiom: `"notify"` says "check the value you sent",
/// `"listen"` / `"watch"` says "check the YAML identifier", etc.
pub(crate) fn constraint_violation_hint(body: &str, subcommand: &str) -> Option<String> {
    let action_suffix = match subcommand {
        "listen" | "watch" => {
            "Check the identifier value in your `--identifiers` JSON (inline mode) or in the listener YAML's `identifiers:` block (YAML mode); run `aviso schema get <TYPE>` for the authoritative schema (handler type and constraints)."
        }
        _ => {
            "Check the value you supplied for this identifier; run `aviso schema get <TYPE>` for the authoritative schema (handler type and constraints)."
        }
    };
    if body.contains("exceeds maximum length") {
        return Some(format!(
            "string identifier value exceeds the schema's max_length constraint (the server names the exact field and limit above). {action_suffix}"
        ));
    }
    if body.contains("Allowed: [") || body.contains("has invalid value") {
        return Some(format!(
            "enum identifier value is not in the schema's allowed set (the server lists the allowed values inline above). {action_suffix}"
        ));
    }
    if body.contains("outside allowed range") {
        return Some(format!(
            "integer identifier value is outside the schema's allowed range (the server lists [min, max] inline above). {action_suffix}"
        ));
    }
    if body.contains("contains invalid date") || body.contains("Failed to parse date") {
        return Some(format!(
            "date identifier value did not parse. The server accepts `YYYY-MM-DD`, `YYYYMMDD`, or `YYYY-DDD` (ISO 8601 ordinal date). {action_suffix}"
        ));
    }
    if body.contains("invalid hours") || body.contains("invalid minutes") {
        return Some(format!(
            "time identifier value out of range. The server expects 4-digit `HHMM` in 24-hour format (e.g. `1200` for noon). {action_suffix}"
        ));
    }
    None
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
            ts = humanise_timestamp(&response.processed_at),
        );
        output::write_stdout_line(&line)
    }
}

/// Splits comma-separated `key=value` and `key:=JSON` parameters with brace
/// and string awareness. Returns the entries in argv order.
fn split_parameters(s: &str) -> Result<Vec<(String, String, bool)>> {
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

fn push_kv(out: &mut Vec<(String, String, bool)>, slice: &str) -> Result<()> {
    let slice = slice.trim();
    if slice.is_empty() {
        return Ok(());
    }
    let eq = slice.find('=').ok_or_else(|| {
        usage_error(format!(
            "parameter parse: no `=` in entry `{slice}` (expected key=value or key:=JSON)"
        ))
    })?;
    let explicit_json = slice[..eq].ends_with(':');
    let key_end = if explicit_json { eq - 1 } else { eq };
    let key = slice[..key_end].trim().to_string();
    if key.is_empty() {
        return Err(usage_error(format!(
            "parameter parse: empty key in entry `{slice}`"
        )));
    }
    let value = slice[eq + 1..].to_string();
    out.push((key, value, explicit_json));
    Ok(())
}

/// Renders the server's RFC3339 `processed_at` timestamp in a
/// human-readable shape for the TTY response line.
///
/// The server sends nanosecond-precision RFC3339 with a numeric
/// timezone offset (e.g. `2026-05-21T21:13:45.558918505+00:00`),
/// which is correct on the wire but visual noise on a terminal.
/// This helper:
///
/// - drops the fractional-seconds component (sub-second precision
///   is rarely actionable for human-eye verification),
/// - replaces the `T` date/time separator with a space,
/// - normalises the UTC notation (`+00:00` or `Z`) to ` UTC`,
/// - leaves any non-UTC offset (`+02:00`, `-05:00`) verbatim.
///
/// Output for the example above: `2026-05-21 21:13:45 UTC`.
///
/// The pipe / `--json` form continues to emit the raw server
/// timestamp unchanged; machine consumers depend on the full
/// precision and explicit offset format.
///
/// Falls back to the raw input on any shape anomaly so the
/// operator still sees the server's response if the wire format
/// changes unexpectedly.
fn humanise_timestamp(raw: &str) -> String {
    let mut s = raw.to_string();
    if let Some(dot_idx) = s.find('.') {
        let tz_offset = s[dot_idx..].find(['+', '-', 'Z']).map(|i| dot_idx + i);
        match tz_offset {
            Some(tz_idx) => s = format!("{}{}", &s[..dot_idx], &s[tz_idx..]),
            None => s.truncate(dot_idx),
        }
    }
    // Replace ONLY the ISO 8601 date/time T separator (the T that
    // sits between two ASCII digits). A naive `find('T')` would also
    // match the T inside the "UTC" suffix this very function emits,
    // breaking idempotency: humanise(humanise(x)) would corrupt the
    // first T it sees in UTC. The digit-surrounded check anchors the
    // replacement to the wire format's `YYYY-MM-DDTHH:MM:SS` shape.
    let bytes = s.as_bytes();
    let iso_t_idx = bytes.iter().enumerate().find_map(|(i, &b)| {
        if b == b'T'
            && i > 0
            && i + 1 < bytes.len()
            && bytes[i - 1].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
        {
            Some(i)
        } else {
            None
        }
    });
    if let Some(i) = iso_t_idx {
        s.replace_range(i..=i, " ");
    }
    if s.ends_with("+00:00") {
        s.truncate(s.len() - "+00:00".len());
        s.push_str(" UTC");
    } else if s.ends_with('Z') {
        s.truncate(s.len() - 1);
        s.push_str(" UTC");
    }
    s
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

/// Parses an identifier value without changing the legacy scalar convention.
/// A value with explicit structured JSON syntax, such as `[[46,8],[47,9]]`, is
/// decoded. An invalid structured value, such as `[46,]`, is rejected. All
/// other values remain strings after legacy outer-quote stripping.
fn parse_identifier_value(
    key: &str,
    value: &str,
    explicit_json: bool,
) -> Result<serde_json::Value> {
    let trimmed = value.trim();
    if explicit_json && trimmed.is_empty() {
        return Err(usage_error(format!(
            "parameter parse: identifier `{key}:=JSON` is empty; provide a valid JSON value such as null, 12, true, \"text\", [], or {{}}"
        )));
    }
    if explicit_json || trimmed.starts_with('[') || trimmed.starts_with('{') {
        serde_json::from_str(trimmed).map_err(|error| {
            let syntax = if explicit_json { ":=JSON" } else { "structured JSON syntax" };
            usage_error(format!(
                "parameter parse: identifier `{key}` uses {syntax} but is invalid JSON at line {line} column {column}: {error}",
                line = error.line(),
                column = error.column(),
            ))
        })
    } else {
        Ok(serde_json::Value::String(strip_outer_quotes(value)))
    }
}

fn build_request_parts(
    entries: &[(String, String, bool)],
) -> Result<(
    String,
    BTreeMap<String, serde_json::Value>,
    Option<serde_json::Value>,
)> {
    let mut event_type: Option<String> = None;
    let mut payload: Option<serde_json::Value> = None;
    let mut identifier: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (key, value, explicit_json) in entries {
        match key.as_str() {
            "event" => {
                if *explicit_json {
                    return Err(usage_error(
                        "parameter parse: `event` must use `event=<TYPE>`, not `event:=JSON`",
                    ));
                }
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
                if *explicit_json {
                    return Err(usage_error(
                        "parameter parse: `data` already expects JSON; use `data=<JSON>`, not `data:=JSON`",
                    ));
                }
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
                identifier.insert(
                    key.clone(),
                    parse_identifier_value(key, value, *explicit_json)?,
                );
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

    fn parts(
        input: &str,
    ) -> (
        String,
        BTreeMap<String, serde_json::Value>,
        Option<serde_json::Value>,
    ) {
        let entries = split_parameters(input).unwrap();
        build_request_parts(&entries).unwrap()
    }

    #[test]
    fn simple_event_and_identifiers() {
        let (event, ident, payload) = parts("event=mars,class=od,stream=oper");
        assert_eq!(event, "mars");
        assert_eq!(
            ident.get("class").and_then(|value| value.as_str()),
            Some("od")
        );
        assert_eq!(
            ident.get("stream").and_then(|value| value.as_str()),
            Some("oper")
        );
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
            ident.get("polygon").and_then(|value| value.as_str()),
            Some("46,8,46,9,47,9"),
            "polygon value should be the unquoted content; got: {ident:?}"
        );
    }

    #[test]
    fn unquoted_identifier_value_passes_through_unchanged() {
        let entries = split_parameters("event=mars,class=od").unwrap();
        let (_, ident, _) = build_request_parts(&entries).unwrap();
        assert_eq!(
            ident.get("class").and_then(|value| value.as_str()),
            Some("od")
        );
    }

    #[test]
    fn explicit_point_cloud_is_parsed_as_json_array() {
        let (_, identifier, _) = parts("event=observations,point_cloud=[[46,8],[47,9]]");
        assert_eq!(
            identifier.get("point_cloud"),
            Some(&serde_json::json!([[46, 8], [47, 9]]))
        );
    }

    #[test]
    fn explicit_object_is_parsed_as_json_value() {
        let (_, identifier, _) = parts(r#"event=mars,area={"north":47,"south":46}"#);
        assert_eq!(
            identifier.get("area"),
            Some(&serde_json::json!({"north": 47, "south": 46}))
        );
    }

    #[test]
    fn bare_json_scalars_remain_strings() {
        let (_, identifier, _) = parts("event=mars,step=12,enabled=true,missing=null");
        assert_eq!(identifier.get("step"), Some(&serde_json::json!("12")));
        assert_eq!(identifier.get("enabled"), Some(&serde_json::json!("true")));
        assert_eq!(identifier.get("missing"), Some(&serde_json::json!("null")));
    }

    #[test]
    fn explicit_json_supports_every_scalar_shape() {
        let (_, identifier, _) = parts(
            r#"event=mars,count:=12,ratio:=1.5,enabled:=true,missing:=null,label:="archive""#,
        );
        assert_eq!(identifier.get("count"), Some(&serde_json::json!(12)));
        assert_eq!(identifier.get("ratio"), Some(&serde_json::json!(1.5)));
        assert_eq!(identifier.get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(identifier.get("missing"), Some(&serde_json::Value::Null));
        assert_eq!(identifier.get("label"), Some(&serde_json::json!("archive")));
    }

    #[test]
    fn explicit_json_supports_arrays_and_objects() {
        let (_, identifier, _) = parts(r#"event=mars,point:=[46,8],area:={"north":47,"south":46}"#);
        assert_eq!(identifier.get("point"), Some(&serde_json::json!([46, 8])));
        assert_eq!(
            identifier.get("area"),
            Some(&serde_json::json!({"north": 47, "south": 46}))
        );
    }

    #[test]
    fn invalid_explicit_json_scalar_has_precise_error() {
        let entries = split_parameters("event=mars,count:=twelve").unwrap();
        let error = build_request_parts(&entries).unwrap_err().to_string();
        assert!(error.contains("count"), "{error}");
        assert!(error.contains(":=JSON"), "{error}");
        assert!(error.contains("invalid JSON"), "{error}");
        assert!(
            error.contains("line") && error.contains("column"),
            "{error}"
        );
    }

    #[test]
    fn empty_explicit_json_has_actionable_error() {
        let entries = split_parameters("event=mars,count:=").unwrap();
        let error = build_request_parts(&entries).unwrap_err().to_string();
        assert!(error.contains("count:=JSON"), "{error}");
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn reserved_parameters_reject_explicit_json_delimiter() {
        for input in [r#"event:="mars""#, "event=mars,data:={}"] {
            let entries = split_parameters(input).unwrap();
            let error = build_request_parts(&entries).unwrap_err().to_string();
            assert!(error.contains("not") && error.contains(":=JSON"), "{error}");
        }
    }

    #[test]
    fn quoted_array_syntax_remains_a_string() {
        let (_, identifier, _) = parts(r#"event=mars,value="[1,2]""#);
        assert_eq!(identifier.get("value"), Some(&serde_json::json!("[1,2]")));
    }

    #[test]
    fn quoted_bracket_prefixed_bare_value_remains_a_string() {
        let (_, identifier, _) = parts(r#"event=mars,label="[archive]""#);
        assert_eq!(
            identifier.get("label"),
            Some(&serde_json::json!("[archive]"))
        );
    }

    #[test]
    fn quoted_brace_prefixed_bare_value_remains_a_string() {
        let (_, identifier, _) = parts(r#"event=mars,label="{region}""#);
        assert_eq!(
            identifier.get("label"),
            Some(&serde_json::json!("{region}"))
        );
    }

    #[test]
    fn invalid_explicit_json_identifier_errors() {
        let entries = split_parameters("event=mars,point_cloud=[[46,8],[47,]]").unwrap();
        let error = build_request_parts(&entries).unwrap_err();
        assert!(error.to_string().contains("point_cloud"));
        assert!(error.to_string().contains("invalid"));
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
    fn constraint_hint_string_max_length_notify() {
        let err =
            http_err_for_constraint("Field 'class' exceeds maximum length of 2 characters, got: 3");
        let hint = hint_for_client_error(&err).expect("max_length MUST yield a hint");
        assert!(hint.contains("max_length"), "{hint}");
        assert!(hint.contains("aviso schema get"), "{hint}");
        assert!(
            hint.contains("value you supplied"),
            "notify-form hint must address the value the operator supplied, not the YAML: {hint}"
        );
    }

    #[test]
    fn constraint_hint_enum_invalid_value_notify() {
        let err =
            http_err_for_constraint("Field 'domain' has invalid value 'zz'. Allowed: [a, b, c]");
        let hint = hint_for_client_error(&err).expect("enum invalid MUST yield a hint");
        assert!(
            hint.contains("enum") || hint.contains("allowed set"),
            "{hint}"
        );
        assert!(hint.contains("aviso schema get"), "{hint}");
    }

    #[test]
    fn constraint_hint_int_range_notify() {
        let err = http_err_for_constraint(
            "Field 'step' value 100001 is outside allowed range [0, 100000]",
        );
        let hint = hint_for_client_error(&err).expect("int range MUST yield a hint");
        assert!(hint.contains("integer") && hint.contains("range"), "{hint}");
    }

    #[test]
    fn constraint_hint_date_format_notify() {
        let err = http_err_for_constraint(
            "Field 'date' contains invalid date 'not-a-date'. Expected: YYYY-MM-DD, YYYYMMDD, or YYYY-DDD",
        );
        let hint = hint_for_client_error(&err).expect("date format MUST yield a hint");
        assert!(
            hint.contains("YYYY-MM-DD") && hint.contains("YYYYMMDD") && hint.contains("YYYY-DDD"),
            "the date-format hint MUST restate all three accepted forms (the server's wording is authoritative; restating it client-side keeps the hint self-contained even when the operator pipes stderr through grep): {hint}"
        );
    }

    #[test]
    fn constraint_hint_time_invalid_hours_notify() {
        let err = http_err_for_constraint(
            "Field 'time' has invalid hours: 25. Hours must be 0-23 in 24-hour format",
        );
        let hint = hint_for_client_error(&err).expect("time invalid MUST yield a hint");
        assert!(hint.contains("HHMM"), "{hint}");
        assert!(hint.contains("24-hour"), "{hint}");
    }

    #[test]
    fn constraint_hint_returns_none_for_unknown_body() {
        let err = http_err_for_constraint("some unrelated 400 error");
        assert!(
            hint_for_client_error(&err).is_none(),
            "constraint hint dispatcher must NOT fire on unrelated bodies; the generic 'no hint' fall-through is the operator's signal that the error body is the authoritative diagnostic"
        );
    }

    fn http_err_for_constraint(details: &str) -> aviso::ClientError {
        aviso::ClientError::Http {
            status: 400,
            body: format!(
                r#"{{"code":"INVALID_NOTIFICATION_REQUEST","details":"{details}","message":"{details}"}}"#
            ),
            request_id: None,
        }
    }

    #[test]
    fn hint_for_polygon_odd_count_sub_message() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: polygon coordinates must be in lat,lon pairs (got an odd number of values)"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("odd-count polygon must yield a hint");
        assert!(
            hint.contains("lat,lon pairs") && hint.contains("even total count"),
            "specific sub-hint for odd count must name 'lat,lon pairs' and 'even total count': {hint}"
        );
        assert!(
            hint.contains("Example:") && hint.contains("polygon=\"46,8,46,9,47,9,47,8,46,8\""),
            "the notify suffix must end with a copy-pasteable Example using the canonical 4-vertex closed polygon: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_non_numeric_sub_message() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: could not parse latitude 'abc' as a number"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("non-numeric polygon must yield a hint");
        assert!(
            hint.contains("must be a number") || hint.contains("semicolon"),
            "specific sub-hint for non-numeric must call out the numeric requirement OR the comma-vs-semicolon delimiter trap: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_too_few_pairs_sub_message() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: polygon must have at least 4 coordinate pairs (3 unique vertices plus a closing repeat of the first vertex)"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("too-few-pairs polygon must yield a hint");
        assert!(
            hint.contains("at least 4 coordinate pairs"),
            "specific sub-hint for too few pairs must restate the minimum: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_empty_sub_message() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: polygon coordinate string is empty"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("empty polygon must yield a hint");
        assert!(
            hint.contains("cannot be empty"),
            "specific sub-hint for empty polygon must say so explicitly: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_lat_out_of_range_sub_message() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: latitude 91 is outside the valid range [-90, 90]"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("lat-out-of-range MUST yield a hint");
        assert!(
            hint.contains("[-90, 90]"),
            "specific sub-hint for lat-out-of-range must restate the valid latitude range: {hint}"
        );
        assert!(
            hint.contains("[-180, 180]"),
            "specific sub-hint MUST also restate the longitude range (the same kind of mistake commonly applies in the other axis): {hint}"
        );
        assert!(
            hint.contains("lat,lon") && hint.contains("not `lon,lat`"),
            "the LAT/LON ORDER mnemonic must be in the hint; this is the single most common cause of an apparent out-of-range value (operator wrote `lon,lat` and the server interpreted the lon as lat): {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_lon_out_of_range_sub_message_uses_same_hint_as_lat() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: longitude 181 is outside the valid range [-180, 180]"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect("lon-out-of-range MUST yield a hint");
        assert!(
            hint.contains("[-90, 90]") && hint.contains("[-180, 180]"),
            "lat-out-of-range and lon-out-of-range MUST share the same hint variant: the message reads naturally in either direction AND the `lat,lon` order mnemonic is the actionable advice for both: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_lat_negative_out_of_range_sub_message() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: latitude -91 is outside the valid range [-90, 90]"}"#.to_string(),
            request_id: None,
        };
        let hint =
            hint_for_client_error(&err).expect("negative-lat-out-of-range MUST yield a hint");
        assert!(
            hint.contains("[-90, 90]"),
            "the negative-axis case must produce the same range-restating hint as the positive case: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_unknown_sub_message_falls_back_to_generic_polygon_advice() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"details":"field 'polygon' must be a valid polygon: brand new server validation we don't know about yet"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err).expect(
            "any polygon-related error MUST yield at least a generic hint; the canonical server prefix `must be a valid polygon` is the catch-all",
        );
        assert!(
            hint.contains("comma-separated list of lat,lon pairs"),
            "generic fallback must spell out the basic format: {hint}"
        );
        assert!(
            hint.contains("Example:") && hint.contains("polygon=\"46,8,46,9,47,9,47,8,46,8\""),
            "the notify suffix must end with a copy-pasteable Example using the canonical 4-vertex closed polygon: {hint}"
        );
    }

    #[test]
    fn hint_for_unknown_event_type_points_at_configured_list() {
        let err = aviso::ClientError::Http {
            status: 400,
            body: r#"{"code":"UNKNOWN_EVENT_TYPE","configured_event_types":["dissemination","mars","test_polygon"],"message":"unknown event type 'marse'"}"#.to_string(),
            request_id: Some("req-xyz".into()),
        };
        let hint = hint_for_client_error(&err).expect("UNKNOWN_EVENT_TYPE must yield a hint");
        assert!(
            hint.contains("event_type") && hint.contains("not configured"),
            "{hint}"
        );
        assert!(hint.contains("aviso schema list"), "{hint}");
        assert!(
            hint.contains("configured_event_types"),
            "should point operator at the server's configured_event_types field for the authoritative list: {hint}"
        );
    }

    #[test]
    fn hint_for_500_polygon_format_also_fires_status_independent() {
        let err = aviso::ClientError::Http {
            status: 500,
            body: r#"{"details":"field 'polygon' must be a valid polygon: polygon coordinates must be in lat,lon pairs (got an odd number of values)"}"#.to_string(),
            request_id: None,
        };
        let hint = hint_for_client_error(&err)
            .expect("polygon hint must fire on 500 too (server may classify as 500 when caught during processing rather than validation)");
        assert!(
            hint.contains("polygon") || hint.contains("lat,lon"),
            "{hint}"
        );
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

    #[test]
    fn humanise_timestamp_standard_nano_precision_utc() {
        assert_eq!(
            humanise_timestamp("2026-05-21T21:13:45.558918505+00:00"),
            "2026-05-21 21:13:45 UTC"
        );
    }

    #[test]
    fn humanise_timestamp_no_fractional_z_suffix() {
        assert_eq!(
            humanise_timestamp("2026-05-21T21:13:45Z"),
            "2026-05-21 21:13:45 UTC"
        );
    }

    #[test]
    fn humanise_timestamp_milli_fractional_non_utc_offset_preserves_offset() {
        assert_eq!(
            humanise_timestamp("2026-05-21T21:13:45.5+02:00"),
            "2026-05-21 21:13:45+02:00",
            "non-UTC offsets must remain visible so operators in non-UTC environments are not confused about which clock the server is on"
        );
    }

    #[test]
    fn humanise_timestamp_no_timezone_marker_drops_fractional_only() {
        assert_eq!(
            humanise_timestamp("2026-05-21T21:13:45.123"),
            "2026-05-21 21:13:45"
        );
    }

    #[test]
    fn humanise_timestamp_malformed_falls_back_to_raw() {
        let raw = "not-a-timestamp";
        assert_eq!(humanise_timestamp(raw), raw);
    }

    #[test]
    fn humanise_timestamp_already_humanised_idempotent() {
        let already = "2026-05-21 21:13:45 UTC";
        assert_eq!(humanise_timestamp(already), already);
    }
}
