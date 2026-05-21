//! `aviso schema` subcommand: `list` and `get`.
//!
//! `aviso schema list` calls `GET /api/v1/schema` and renders the
//! returned catalogue in a TTY-aware way per Q5: human-readable
//! table on a terminal, NDJSON otherwise (or always when `--json`
//! is set).
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

    if output::use_ndjson(resolved.force_json) {
        for event_type in &catalogue.event_types {
            let row = serde_json::json!({
                "event_type": event_type,
                "schema": catalogue
                    .schema
                    .get(event_type)
                    .map(stream_schema_to_value),
            });
            output::write_stdout_line(&serde_json::to_string(&row)?)?;
        }
        Ok(())
    } else {
        write_table(&catalogue)
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
    let pretty = serde_json::to_string_pretty(&value)?;
    output::write_stdout_bytes(pretty.as_bytes())?;
    output::write_stdout_bytes(b"\n")
}

fn stream_schema_to_value(schema: &aviso::StreamSchema) -> serde_json::Value {
    serde_json::json!({
        "payload": schema.payload,
        "identifier": schema.identifier,
    })
}

fn write_table(catalogue: &aviso::SchemaCatalog) -> Result<()> {
    output::write_stdout_line(&format!(
        "{count} schema(s) registered (status: {status})",
        count = catalogue.total_schemas,
        status = catalogue.status
    ))?;
    let mut rows: Vec<&String> = catalogue.event_types.iter().collect();
    rows.sort();
    for event_type in rows {
        output::write_stdout_line(&format!("- {event_type}"))?;
    }
    Ok(())
}
