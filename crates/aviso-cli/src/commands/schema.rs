//! `aviso schema` subcommand: `list` and `get`.
//!
//! `aviso schema list` calls `GET /api/v1/schema` and renders the
//! registered event-type names. The TTY and NDJSON forms carry
//! the SAME content (just event-type names) so that piping does
//! not silently change what the operator sees; the only difference
//! is rendering. Operators who want a full schema use
//! `aviso schema get <EVENT_TYPE>` for one entry, or pipe the
//! `list` output through `xargs -I{} aviso schema get {}` for all.
//! This matches the `ls` / `git branch` convention where the
//! listing command is an index and a separate command reads details.
//!
//! `aviso schema get <EVENT_TYPE>` calls `GET /api/v1/schema/{type}`
//! and pretty-prints the JSON document. Schemas are JSON by nature
//! so there is no "human-readable text" alternative; `--json` is
//! accepted but is a no-op for the get path.

use anyhow::Result;

use crate::client_builder;
use crate::config::Resolved;
use crate::output;

/// Runs `aviso schema list`.
pub(crate) async fn run_list(resolved: &Resolved) -> Result<()> {
    let client = client_builder::build(resolved, None, false)?;
    let catalogue = match client.schema().await {
        Ok(c) => c,
        Err(client_err) => return Err(wrap_error(client_err, "list", None)),
    };

    let mut event_types: Vec<&String> = catalogue.event_types.iter().collect();
    event_types.sort();

    if output::use_ndjson(resolved.force_json) {
        for event_type in event_types {
            let row = serde_json::json!({ "event_type": event_type });
            output::write_stdout_line(&serde_json::to_string(&row)?)?;
        }
        Ok(())
    } else {
        write_table(&catalogue, &event_types)
    }
}

/// Runs `aviso schema get <EVENT_TYPE>`.
pub(crate) async fn run_get(resolved: &Resolved, event_type: &str) -> Result<()> {
    let client = client_builder::build(resolved, None, false)?;
    let response = match client.schema_for(event_type).await {
        Ok(r) => r,
        Err(client_err) => return Err(wrap_error(client_err, "get", Some(event_type))),
    };
    let value = serde_json::json!({
        "status": response.status,
        "event_type": response.event_type,
        "schema": stream_schema_to_value(&response.schema),
    });
    let mut pretty = serde_json::to_string_pretty(&value)?;
    pretty.push('\n');
    output::write_stdout_bytes(pretty.as_bytes())
}

/// Wraps a [`aviso::ClientError`] from the schema endpoints with the
/// canonical `GET /api/v1/schema...` context line AND a `suggestion:`
/// context line when [`hint_for_client_error`] recognises an
/// operator-mistake pattern. Mirrors the per-subcommand pattern in
/// `commands::notify::run` and `commands::listen::drive`; centralised
/// here because `schema list` and `schema get` share the same hint
/// surface (404 + available_types, 401 credentials, 403 permission).
fn wrap_error(
    client_err: aviso::ClientError,
    operation: &str,
    event_type: Option<&str>,
) -> anyhow::Error {
    let hint = hint_for_client_error(&client_err);
    let path = match event_type {
        Some(t) => format!("/api/v1/schema/{t}"),
        None => "/api/v1/schema".to_string(),
    };
    let mut err =
        anyhow::Error::from(client_err).context(format!("GET {path} (schema {operation})"));
    if let Some(h) = hint {
        err = err.context(format!("suggestion: {h}"));
    }
    err
}

/// Inspects a [`aviso::ClientError`] for known schema-endpoint
/// operator-mistake patterns and returns a one-line hint when matched.
///
/// The 404 + `available_types` pattern is the load-bearing case:
/// `aviso schema get <wrong_name>` is the most common entry path for
/// a typo (operators discover event types via `schema list`, and a
/// typo in the subsequent `schema get` is the moment we can be most
/// helpful by NAMING THE ACTUAL TYPES instead of just saying "not
/// found"). Parses the JSON body to extract the server-supplied
/// `available_types` array verbatim; falls back to a generic
/// "run `aviso schema list`" hint when the body shape is unexpected
/// so the hint never disappears on a future server response format
/// change.
fn hint_for_client_error(err: &aviso::ClientError) -> Option<String> {
    let aviso::ClientError::Http { status, body, .. } = err else {
        return None;
    };
    if *status == 404 && body.contains("available_types") {
        let available = parse_available_types(body);
        return Some(if let Some(list) = available {
            format!(
                "the event_type is not registered on this server. Available event_types: {list}. Run `aviso schema list` for the same list."
            )
        } else {
            "the event_type is not registered on this server. Run `aviso schema list` for the available types.".to_string()
        });
    }
    match *status {
        401 => Some(
            "credentials are missing, invalid, or expired. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars; verify auth wired up via `aviso config dump --redact`."
                .to_string(),
        ),
        403 => Some(
            "credentials were accepted but may not have permission to read schemas. Contact the server admin."
                .to_string(),
        ),
        _ => None,
    }
}

/// Extracts the `available_types` array from a schema 404 body into
/// a comma-separated string suitable for embedding in the hint.
/// Returns `None` when the body is not valid JSON, the field is
/// missing, the field is not a JSON array, or every element fails
/// the string coercion (defensive against server format drift).
fn parse_available_types(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = parsed.get("available_types")?.as_array()?;
    let mut types: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if types.is_empty() {
        return None;
    }
    types.sort();
    Some(types.join(", "))
}

fn stream_schema_to_value(schema: &aviso::StreamSchema) -> serde_json::Value {
    serde_json::json!({
        "payload": schema.payload,
        "identifier": schema.identifier,
    })
}

fn write_table(catalogue: &aviso::SchemaCatalog, event_types: &[&String]) -> Result<()> {
    output::write_stdout_line(&format!(
        "{count} schema(s) registered (status: {status})",
        count = catalogue.total_schemas,
        status = catalogue.status
    ))?;
    for event_type in event_types {
        output::write_stdout_line(&format!("- {event_type}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic ClientError fixtures is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn http_err(status: u16, body: &str) -> aviso::ClientError {
        aviso::ClientError::Http {
            status,
            body: body.to_string(),
            request_id: Some("req-test".into()),
        }
    }

    #[test]
    fn hint_for_404_unknown_event_type_names_the_available_types_inline() {
        let body = r#"{"available_types":["mars","test_polygon","dissemination"],"message":"Event type 'not_a_real_event' not found","status":"error"}"#;
        let hint = hint_for_client_error(&http_err(404, body))
            .expect("404 + available_types MUST yield a hint; this is the load-bearing case for schema get typos");
        assert!(
            hint.contains("not registered"),
            "hint should say the type is not registered (not just 'not found' which is the server's wording): {hint}"
        );
        assert!(
            hint.contains("dissemination, mars, test_polygon"),
            "hint MUST embed the available_types list ALPHABETICALLY SORTED so the operator sees the actual options inline: {hint}"
        );
        assert!(
            hint.contains("aviso schema list"),
            "hint MUST also point at the listing command for the same data on demand: {hint}"
        );
    }

    #[test]
    fn hint_for_404_with_unparseable_body_still_yields_a_generic_hint() {
        let hint = hint_for_client_error(&http_err(404, "available_types but not valid JSON {{{"))
            .expect("any 404 + 'available_types' marker MUST still produce SOMETHING actionable; the body parse failure must NOT cause the hint to silently disappear");
        assert!(
            hint.contains("aviso schema list"),
            "generic fallback MUST point at the listing command: {hint}"
        );
        assert!(
            !hint.contains(", "),
            "generic fallback must NOT pretend to list types when the body parse failed: {hint}"
        );
    }

    #[test]
    fn hint_for_401_credentials_consistent_with_notify_listen() {
        let hint = hint_for_client_error(&http_err(401, "{}"))
            .expect("401 MUST yield a credentials hint consistent with notify and listen");
        assert!(hint.contains("credentials"), "{hint}");
        assert!(hint.contains("config dump"), "{hint}");
    }

    #[test]
    fn hint_for_403_permission_specific_to_schema_reading() {
        let hint = hint_for_client_error(&http_err(403, "{}"))
            .expect("403 MUST yield a permission hint scoped to schema reading");
        assert!(
            hint.contains("read schemas"),
            "the 403 hint MUST name the SPECIFIC permission (not 'watch' or 'notify' which would mislead): {hint}"
        );
    }

    #[test]
    fn hint_for_unknown_http_status_returns_none() {
        assert!(hint_for_client_error(&http_err(502, "<html>...</html>")).is_none());
    }

    #[test]
    fn hint_for_non_http_client_error_returns_none() {
        let err = aviso::ClientError::Auth("test".into());
        assert!(hint_for_client_error(&err).is_none());
    }

    #[test]
    fn parse_available_types_sorts_alphabetically_for_deterministic_hints() {
        let body = r#"{"available_types":["zebra","apple","mars"]}"#;
        assert_eq!(
            parse_available_types(body).unwrap(),
            "apple, mars, zebra",
            "the alphabetical sort MUST happen here (not at the call site) so every hint variant gets a stable order"
        );
    }

    #[test]
    fn parse_available_types_returns_none_on_empty_array() {
        let body = r#"{"available_types":[]}"#;
        assert!(
            parse_available_types(body).is_none(),
            "an empty array MUST produce None so the caller falls back to the generic hint rather than emitting an empty 'Available event_types: .' line"
        );
    }

    #[test]
    fn parse_available_types_returns_none_on_missing_field() {
        let body = r#"{"message":"some other error"}"#;
        assert!(parse_available_types(body).is_none());
    }

    #[test]
    fn parse_available_types_returns_none_on_non_array_field() {
        let body = r#"{"available_types":"not_an_array"}"#;
        assert!(
            parse_available_types(body).is_none(),
            "a server format drift (string instead of array) MUST fall through to the generic hint, not silently render 'Available event_types: not_an_array'"
        );
    }
}
