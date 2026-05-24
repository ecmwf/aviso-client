# Discovering schemas

`aviso-server` stores one schema per event type, describing the valid identifier fields and any payload constraints. The client exposes two read-only methods over `GET /api/v1/schema` for tooling that wants to enumerate or inspect them.

The client does **not** validate notifications against schemas. The server remains the single source of truth; this module is pass-through only.

## List every schema

```rust,ignore
use aviso::AvisoClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("http://localhost:8000")
        .build()?;

    let catalog = client.schema().await?;
    println!("{} schemas:", catalog.total_schemas);
    for name in &catalog.event_types {
        println!("  {name}");
    }
    Ok(())
}
```

## Fetch a single schema

```rust,ignore
use aviso::AvisoClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("http://localhost:8000")
        .build()?;

    let response = client.schema_for("mars").await?;
    for (field_name, rule) in &response.schema.identifier {
        println!("identifier.{field_name} = {rule}");
    }
    Ok(())
}
```

A missing event type returns `ClientError::Http` with `status = 404` and the server-supplied body.

## Permissive types

`StreamSchema` stores identifier rules and the payload configuration as `serde_json::Value`. This is deliberate: the server can grow new fields without a client change. If you need a typed view of a specific rule (for example to render it in a CLI), pattern-match the `Value` yourself.

## Refresh-on-401

Both `schema` and `schema_for` go through the same 401-refresh-retry-once path as `notify`. A `401` on a schema GET against a configured auth provider refreshes credentials and retries once.
