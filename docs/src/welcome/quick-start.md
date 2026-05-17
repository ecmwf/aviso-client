# Quick start

This page walks through the smallest useful Rust program that talks to an `aviso-server`.

## Add the dependency

```toml
[dependencies]
aviso = "0.1"
tokio = { version = "1.45", features = ["macros", "rt-multi-thread"] }
```

`aviso` is async and uses `tokio` as the runtime.

## Build a client

```rust,ignore
use aviso::AvisoClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("http://localhost:8000")
        .build()?;

    println!("client base url = {}", client.base_url());
    Ok(())
}
```

The client is `Clone`. Cloned handles share the same underlying HTTP connection pool and authentication provider, so you can hand copies to multiple tasks without paying for extra sockets.

## Authentication

The example above sends no `Authorization` header, which is fine for anonymous streams. For authenticated servers, attach a provider:

```rust,ignore
use std::sync::Arc;
use aviso::{AvisoClient, auth::Bearer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = Arc::new(Bearer::new("opaque-or-jwt-token")?);
    let client = AvisoClient::builder()
        .base_url("https://aviso.example.org")
        .auth(auth)
        .build()?;
    println!("client ready, base = {}", client.base_url());
    Ok(())
}
```

The authentication chapter covers the five shipped providers: `Basic`, `Bearer`, `Env`, `ConfigFile`, and `Chain`.

## Base URL handling

The builder normalizes the base URL at build time:

- A trailing slash is added if missing (`http://localhost:8000` becomes `http://localhost:8000/`).
- A path prefix is preserved (`https://gw.example.org/aviso` becomes `https://gw.example.org/aviso/`), so the client works behind a reverse proxy that mounts `aviso-server` under a sub-path.

Endpoint paths are always joined as relative (`api/v1/notification`), never as absolute (`/api/v1/notification`); the absolute form would strip the proxy prefix.
