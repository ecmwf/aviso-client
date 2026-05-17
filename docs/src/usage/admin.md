# Admin operations

`aviso-server` exposes three operator-level deletion endpoints under `/api/v1/admin/`. The client wraps each one as a plain method on `AvisoClient`. There is no built-in `--yes` confirmation; CLI or higher-level tooling layered on top is expected to add its own.

All three operations require operator-level authentication; expect `403 Forbidden` (returned as `ClientError::Http`) when called with insufficient privileges.

## Wipe a single stream

```rust,ignore
use std::sync::Arc;

use aviso::{AvisoClient, auth::Bearer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("https://aviso.example.org")
        .auth(Arc::new(Bearer::new("operator-token")?))
        .build()?;

    client.wipe_stream("mars").await?;
    Ok(())
}
```

The server expects the stream name in the request body; the client handles that automatically.

## Wipe every stream

```rust,ignore
client.wipe_all().await?;
```

This deletes notifications for every configured stream. Wrap it in your own confirmation step before calling.

## Delete a single notification

```rust,ignore
// The id is the same <event_type>@<sequence> form the server emits in CloudEvents.
client.delete_notification("mars@42").await?;
```

The `@` character in the id is sent verbatim because the server's route matches on the literal id. Server-generated ids never contain `/`, so the path is always single-segment.

## Refresh-on-401

All three methods go through the shared `401 -> refresh -> retry once` path. A `401` against a configured auth provider triggers `AuthProvider::refresh` and one retry; a `401` without auth, or a second `401`, surfaces immediately as `ClientError::Http`.
