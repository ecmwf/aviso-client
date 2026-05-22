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
    let augmented = match &err {
        ClientError::Http { status: 401, .. } => {
            "auth failed: admin endpoints require valid credentials with admin role. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars."
        }
        ClientError::Http { status: 403, .. } => {
            "forbidden: credentials valid but lack admin role. Ask the aviso-server operator to grant the admin role on this principal."
        }
        _ => return anyhow::Error::from(err).context(format!("admin {operation}")),
    };
    anyhow::Error::from(err).context(augmented.to_string())
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
