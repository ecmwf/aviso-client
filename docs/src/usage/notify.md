# Publishing notifications

`AvisoClient::notify` publishes a single notification to `POST /api/v1/notification`. It returns a [`NotifyResponse`](https://docs.rs/aviso/latest/aviso/struct.NotifyResponse.html) carrying the server's status, request id, and processing timestamp.

## Minimal example

```rust,ignore
use std::collections::BTreeMap;
use std::sync::Arc;

use aviso::{AvisoClient, NotificationRequest, auth::Bearer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("https://aviso.example.org")
        .auth(Arc::new(Bearer::new("opaque-jwt")?))
        .build()?;

    let mut identifier = BTreeMap::new();
    identifier.insert("class".into(), "od".into());
    identifier.insert("stream".into(), "oper".into());

    let request = NotificationRequest::new("mars")
        .with_identifier(identifier)
        .with_payload(serde_json::json!({ "location": "s3://bucket/path" }));

    let response = client.notify(&request).await?;
    println!(
        "published: request_id={}, processed_at={}",
        response.request_id, response.processed_at
    );
    Ok(())
}
```

## Refresh-on-401

On a `401 Unauthorized` response, `notify` calls `AuthProvider::refresh` on the configured provider and retries the request **once**. A second `401` is returned to the caller as `ClientError::Http`; the client never loops on auth failures.

The shipped static-credential providers (`Basic`, `Bearer`) treat refresh as a no-op, which is correct: a `401` against a static credential means the credential is wrong. Custom providers backed by a cache (OAuth, OIDC, signed-URL) override `refresh` to rotate their cached token.

## Retry policy on transport errors

Per the project's error design, `notify` does not retry on a transport error after the request body has been sent. The server may have processed the publish, so a blind retry would risk a duplicate. The decision is recorded in [D16](../internals/decisions.md). When a server-side idempotency-key contract becomes available the policy will be revisited.

## Error variants

| Variant | When |
| --- | --- |
| `ClientError::Transport` | DNS, connect, TLS, or partial body before any response |
| `ClientError::Http` | Any non-success status; carries `status`, `body`, and `X-Request-ID` |
| `ClientError::Decode` | Body did not deserialize as `NotifyResponse` (server contract drift) |
| `ClientError::Auth` | Auth provider failed to produce a header, or `refresh` itself failed |

Log `request_id` from successful responses and from `ClientError::Http` errors. ECMWF support traces are indexed by it.