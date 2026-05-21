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

use anyhow::{Context, Result};

use crate::client_builder;
use crate::config::Resolved;
use crate::output;

/// Runs `aviso schema list`.
pub(crate) async fn run_list(resolved: &Resolved) -> Result<()> {
    let client = client_builder::build(resolved, None)?;
    let catalogue = client.schema().await.context("GET /api/v1/schema")?;

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
    let client = client_builder::build(resolved, None)?;
    let response = client
        .schema_for(event_type)
        .await
        .with_context(|| format!("GET /api/v1/schema/{event_type}"))?;
    let value = serde_json::json!({
        "status": response.status,
        "event_type": response.event_type,
        "schema": stream_schema_to_value(&response.schema),
    });
    let mut pretty = serde_json::to_string_pretty(&value)?;
    pretty.push('\n');
    output::write_stdout_bytes(pretty.as_bytes())
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
