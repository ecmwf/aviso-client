//! `aviso admin` subcommand family.
//!
//! Three destructive operations sharing the same shape: each leaf
//! requires `--yes` (gated in `main.rs::run` before dispatch here),
//! calls the corresponding lib method, and emits a Q5-compliant
//! confirmation line. Auth failures surface with operator-friendly
//! stderr messages distinguishing 401 (bad credentials) from 403
//! (valid credentials, missing admin role).

use anyhow::{Context, Result};
use aviso::ClientError;

use crate::client_builder;
use crate::config::Resolved;
use crate::output;

/// Runs `aviso admin wipe-stream <EVENT_TYPE> --yes`.
pub(crate) async fn run_wipe_stream(resolved: &Resolved, event_type: &str) -> Result<()> {
    let client = client_builder::build(resolved, None, false)?;
    client
        .wipe_stream(event_type)
        .await
        .map_err(|e| augment_admin_error(e, "wipe-stream"))?;
    write_ok(resolved, "wipe_stream", &[("event_type", event_type)])
}

/// Runs `aviso admin wipe-all --yes`.
pub(crate) async fn run_wipe_all(resolved: &Resolved) -> Result<()> {
    let client = client_builder::build(resolved, None, false)?;
    client
        .wipe_all()
        .await
        .map_err(|e| augment_admin_error(e, "wipe-all"))?;
    write_ok(resolved, "wipe_all", &[])
}

/// Runs `aviso admin delete <NOTIFICATION_ID> --yes`.
pub(crate) async fn run_delete(resolved: &Resolved, notification_id: &str) -> Result<()> {
    let client = client_builder::build(resolved, None, false)?;
    client
        .delete_notification(notification_id)
        .await
        .map_err(|e| augment_admin_error(e, "delete"))?;
    write_ok(
        resolved,
        "delete_notification",
        &[("notification_id", notification_id)],
    )
}

fn augment_admin_error(err: ClientError, operation: &str) -> anyhow::Error {
    let hint: Option<String> = if let ClientError::Http { status, body, .. } = &err {
        match *status {
            401 => Some(
                "auth failed: admin endpoints require valid credentials with admin role. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars.".to_string(),
            ),
            403 => Some(
                "forbidden: credentials valid but lack admin role. Ask the aviso-server operator to grant the admin role on this principal.".to_string(),
            ),
            400 if body.contains("'<stream>@<sequence>' format") || body.contains("must be in") => Some(
                "the notification id is malformed. The id format is `<event_type>@<sequence>` (example: `mars@72`).".to_string(),
            ),
            404 if body.contains("Notification not found") => Some(
                "the event_type in the notification id is not registered on this server. The id format is `<event_type>@<sequence>`; run `aviso schema list` for valid event_types.".to_string(),
            ),
            500 if body.contains("Failed to delete sequence") => Some(
                "the server accepted the event_type but could not find the sequence number. The id format is `<event_type>@<sequence>`; verify the sequence number references a notification that currently exists on the server.".to_string(),
            ),
            _ => None,
        }
    } else {
        None
    };
    let mut augmented = anyhow::Error::from(err).context(format!("admin {operation}"));
    if let Some(suggestion) = hint {
        augmented = augmented.context(format!("suggestion: {suggestion}"));
    }
    augmented
}

fn write_ok(resolved: &Resolved, operation: &str, fields: &[(&str, &str)]) -> Result<()> {
    if output::use_ndjson(resolved.force_json) {
        let mut value = serde_json::Map::new();
        value.insert("status".into(), serde_json::Value::String("ok".into()));
        value.insert(
            "operation".into(),
            serde_json::Value::String(operation.into()),
        );
        for (k, v) in fields {
            value.insert(
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            );
        }
        output::write_stdout_line(
            &serde_json::to_string(&serde_json::Value::Object(value))
                .context("serialise admin response")?,
        )
    } else {
        let detail = fields
            .iter()
            .map(|(k, v)| format!("{k}=`{v}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let line = if detail.is_empty() {
            format!("ok: {operation} succeeded")
        } else {
            format!("ok: {operation} succeeded ({detail})")
        };
        output::write_stdout_line(&line)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic ClientError fixtures is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn http_err(status: u16, body: &str) -> ClientError {
        ClientError::Http {
            status,
            body: body.to_string(),
            request_id: Some("req-test".into()),
        }
    }

    #[test]
    fn augment_admin_error_404_notification_not_found_appends_event_type_hint() {
        let err = http_err(
            404,
            r#"{"success":false,"message":"Notification not found"}"#,
        );
        let augmented = augment_admin_error(err, "delete");
        let chain: Vec<String> = augmented.chain().map(ToString::to_string).collect();
        assert!(
            chain
                .iter()
                .any(|s| s.contains("suggestion:") && s.contains("`<event_type>@<sequence>`")),
            "404 + Notification not found MUST produce a suggestion: context naming the id format. Chain: {chain:?}",
        );
        assert!(
            chain
                .iter()
                .any(|s| s.contains("event_type") && s.contains("not registered")),
            "the hint MUST identify the event_type as the specific failing part (the server returns 404 specifically when the stream does not exist; missing sequences on a valid stream produce 500 instead, so the hint must NOT lump both cases together): {chain:?}",
        );
        assert!(
            chain.iter().any(|s| s.contains("aviso schema list")),
            "the hint MUST point at `aviso schema list` for the authoritative event_type list: {chain:?}",
        );
        assert!(
            chain.iter().any(|s| s == "admin delete"),
            "the operation context `admin delete` MUST be present in the chain alongside the suggestion so `format_chain` renders `error: admin delete` (not `error: http 404 ...`) as the summary, matching the notify/listen pattern: {chain:?}",
        );
        let chain_with_suggestion_idx = chain
            .iter()
            .position(|s| s.starts_with("suggestion: "))
            .unwrap();
        let chain_with_op_idx = chain.iter().position(|s| s == "admin delete").unwrap();
        assert!(
            chain_with_suggestion_idx < chain_with_op_idx,
            "the suggestion MUST be added AFTER the operation context (so it appears closer to the chain head); chain order: {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_500_missing_sequence_appends_sequence_hint() {
        let err = http_err(
            500,
            r#"{"success":false,"message":"Failed to delete notification: Failed to delete sequence 99999999 from stream MARS","notification_id":"mars@99999999"}"#,
        );
        let chain: Vec<String> = augment_admin_error(err, "delete")
            .chain()
            .map(ToString::to_string)
            .collect();
        assert!(
            chain
                .iter()
                .any(|s| s.contains("suggestion:") && s.contains("sequence")),
            "500 + 'Failed to delete sequence' MUST produce a sequence-focused suggestion: {chain:?}",
        );
        assert!(
            chain
                .iter()
                .any(|s| s.contains("event_type") && s.contains("accepted")),
            "the hint MUST clarify that the event_type IS valid (the server accepted it) so the operator focuses on the sequence rather than the event_type: {chain:?}",
        );
        assert!(
            !chain.iter().any(|s| s.contains("aviso schema list")),
            "the 500-sequence hint MUST NOT point at `aviso schema list` (the event_type is valid; the operator's problem is the sequence number, not the schema): {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_400_malformed_id_appends_format_hint() {
        let err = http_err(
            400,
            r#"{"success":false,"message":"notification_id must be in '<stream>@<sequence>' format"}"#,
        );
        let chain: Vec<String> = augment_admin_error(err, "delete")
            .chain()
            .map(ToString::to_string)
            .collect();
        assert!(
            chain
                .iter()
                .any(|s| s.contains("suggestion:") && s.contains("malformed")),
            "400 + 'must be in <stream>@<sequence>' MUST produce a format-focused suggestion: {chain:?}",
        );
        assert!(
            chain
                .iter()
                .any(|s| s.contains("`mars@72`") || s.contains("example:")),
            "the hint MUST include a copy-pasteable example so the operator sees the expected shape concretely: {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_401_auth_hint() {
        let err = http_err(401, "{}");
        let chain: Vec<String> = augment_admin_error(err, "delete")
            .chain()
            .map(ToString::to_string)
            .collect();
        assert!(
            chain.iter().any(|s| s.contains("admin role")),
            "401 hint MUST specifically mention admin role (NOT generic 'credentials' which is the notify/listen wording for non-admin endpoints): {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_403_admin_role_hint() {
        let err = http_err(403, "{}");
        let chain: Vec<String> = augment_admin_error(err, "wipe-stream")
            .chain()
            .map(ToString::to_string)
            .collect();
        assert!(
            chain.iter().any(|s| s.contains("admin role")),
            "403 hint MUST specifically mention admin role: {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_404_unrelated_body_falls_through_to_operation_context() {
        let err = http_err(404, r#"{"something else entirely":"true"}"#);
        let chain: Vec<String> = augment_admin_error(err, "delete")
            .chain()
            .map(ToString::to_string)
            .collect();
        assert!(
            chain.iter().any(|s| s.contains("admin delete")),
            "404 with unrelated body MUST fall through to the generic `admin {{operation}}` context (no spurious suggestion): {chain:?}",
        );
        assert!(
            !chain.iter().any(|s| s.contains("suggestion:")),
            "no `suggestion:` context expected for unrecognised 404 body: {chain:?}",
        );
    }
}
