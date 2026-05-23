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
    if let ClientError::Http { status, body, .. } = &err {
        let hint = match *status {
            401 => Some(
                "auth failed: admin endpoints require valid credentials with admin role. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars.".to_string(),
            ),
            403 => Some(
                "forbidden: credentials valid but lack admin role. Ask the aviso-server operator to grant the admin role on this principal.".to_string(),
            ),
            404 if body.contains("Notification not found") => Some(
                "notification id not found. The id format is `<event_type>@<sequence>`; check for typos in either part. Run `aviso schema list` for valid event_types; note the server returns the same 404 whether the event_type is wrong or the sequence does not exist on that stream.".to_string(),
            ),
            _ => None,
        };
        if let Some(suggestion) = hint {
            return anyhow::Error::from(err).context(format!("suggestion: {suggestion}"));
        }
    }
    anyhow::Error::from(err).context(format!("admin {operation}"))
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
    fn augment_admin_error_404_notification_not_found_appends_hint() {
        let err = http_err(404, r#"{"success":false,"message":"Notification not found"}"#);
        let augmented = augment_admin_error(err, "delete");
        let chain: Vec<String> = augmented.chain().map(|e| e.to_string()).collect();
        assert!(
            chain.iter().any(|s| s.contains("suggestion:") && s.contains("`<event_type>@<sequence>`")),
            "404 + Notification not found MUST produce a suggestion: context naming the id format. Chain: {chain:?}",
        );
        assert!(
            chain.iter().any(|s| s.contains("aviso schema list")),
            "the hint MUST point at `aviso schema list` for the authoritative event_type list: {chain:?}",
        );
        assert!(
            chain.iter().any(|s| s.contains("the same 404") || s.contains("indistinguishable")),
            "the hint MUST tell the operator that wrong-event_type and wrong-sequence produce the SAME 404 (the disambiguation is the operator's responsibility): {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_401_auth_hint() {
        let err = http_err(401, r#"{}"#);
        let chain: Vec<String> = augment_admin_error(err, "delete").chain().map(|e| e.to_string()).collect();
        assert!(
            chain.iter().any(|s| s.contains("admin role")),
            "401 hint MUST specifically mention admin role (NOT generic 'credentials' which is the notify/listen wording for non-admin endpoints): {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_403_admin_role_hint() {
        let err = http_err(403, r#"{}"#);
        let chain: Vec<String> = augment_admin_error(err, "wipe-stream").chain().map(|e| e.to_string()).collect();
        assert!(
            chain.iter().any(|s| s.contains("admin role")),
            "403 hint MUST specifically mention admin role: {chain:?}",
        );
    }

    #[test]
    fn augment_admin_error_404_unrelated_body_falls_through_to_operation_context() {
        let err = http_err(404, r#"{"something else entirely":"true"}"#);
        let chain: Vec<String> = augment_admin_error(err, "delete").chain().map(|e| e.to_string()).collect();
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
