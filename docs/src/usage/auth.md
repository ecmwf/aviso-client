# Authentication

`aviso-server` accepts three authentication modes: anonymous, HTTP Basic, and Bearer (JWT or opaque). The client surfaces these as the `AuthProvider` async trait and five shipped implementations.

The provider is optional on `AvisoClient`. If you do not set one, the client sends no `Authorization` header, which is the right shape for anonymous streams.

## The shipped providers

| Provider | What it sends | Where the credentials come from |
| --- | --- | --- |
| `Basic` | `Authorization: Basic <base64(user:pass)>` | Explicit constructor |
| `Bearer` | `Authorization: Bearer <token>` | Explicit constructor |
| `Env` | Basic or Bearer | `AVISO_TOKEN`, `AVISO_USERNAME`, `AVISO_PASSWORD` from the process environment |
| `ConfigFile` | Basic or Bearer | A YAML file you supply |
| `Chain` | First member's header that succeeds | Composition of any of the above |

All five mark the returned `Authorization` header as sensitive via `HeaderValue::set_sensitive(true)`, so `reqwest`, `hyper`, and other downstream debug/log paths redact the value.

## Basic and Bearer

```rust,ignore
use aviso::auth::{Basic, Bearer};

let basic = Basic::new("alice", "wonderland")?;
let bearer = Bearer::new("opaque-or-jwt-token")?;
```

Constructors are fallible. `Basic::new` rejects an empty username or a username containing `:` (the user/password separator per RFC 7617). `Bearer::new` rejects an empty token. Both reject at construction time, not at first request, so configuration errors surface immediately.

The `Debug` impl on both types redacts the secret:

```text
Basic { user: "alice", pass: "<redacted>" }
Bearer { token: "<redacted>" }
```

## Env

`Env` reads credentials from the process environment. Resolution at construction:

1. If `AVISO_TOKEN` is set and non-empty, use Bearer.
2. Otherwise, if both `AVISO_USERNAME` and `AVISO_PASSWORD` are set (username non-empty), use Basic.
3. Otherwise, return `ClientError::Auth` describing what was missing.

```rust,ignore
use aviso::auth::Env;

let env = Env::from_process_env()?;
```

The variable name `AVISO_TOKEN` matches the legacy `pyaviso` convention so existing operator scripts work without renaming.

## ConfigFile

`ConfigFile` reads a YAML file with exactly one of two shapes:

```yaml
bearer:
  token: "opaque-or-jwt-token"
```

or:

```yaml
basic:
  username: "alice"
  password: "wonderland"
```

```rust,ignore
use aviso::auth::ConfigFile;

let cfg = ConfigFile::from_path("/etc/aviso/auth.yaml")?;
```

The parser is strict: unknown top-level keys (`beare:` instead of `bearer:`) and unknown inner-section keys (`tokn:` instead of `token:`) are rejected so typos surface immediately. Specifying both sections, or neither, also produces an error.

For in-memory configs (tests, remote-fetched config), `ConfigFile::from_yaml_str(yaml: &str)` is the testable kernel and accepts the same shape.

## Chain

`Chain` composes multiple providers. Each member is tried in order; the first successful `Authorization` header wins. If every member fails, the last error is returned.

`Chain` is a runtime fallback over already-constructed providers, not a source-discovery mechanism. Build it from the sources that actually constructed successfully:

```rust,ignore
use std::sync::Arc;
use aviso::auth::{AuthProvider, Chain, ConfigFile, Env};

let mut providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
if let Ok(env) = Env::from_process_env() {
    providers.push(Arc::new(env));
}
if let Ok(file) = ConfigFile::from_path("/etc/aviso/auth.yaml") {
    providers.push(Arc::new(file));
}
let chain = Chain::new(providers);
```

## Refresh on 401

The trait exposes a `refresh()` method that the client calls on a `401 Unauthorized` response (per [D8](../internals/decisions.md)). The shipped static-credential providers treat refresh as a no-op: a `401` against a static credential means the credential is wrong and refreshing changes nothing. `Chain` overrides refresh to fan out to every member and returns `Ok` if at least one member's refresh succeeded.

Providers that hold cached tokens (OAuth, OIDC, signed-URL) override `refresh()` to rotate their cached token. Because the trait is taken by shared reference (so the same provider can be cloned through `Arc<dyn AuthProvider>` across tasks), refresh implementations use interior mutability (`Mutex`, `RwLock`, `tokio::sync::RwLock`, or an atomic) to publish the new token. The next call to `authorization_header()` is expected to see it.

## Writing a custom provider

```rust,ignore
use async_trait::async_trait;
use reqwest::header::HeaderValue;
use tokio::sync::RwLock;

use aviso::auth::AuthProvider;
use aviso::ClientError;

struct MyOauth {
    cached_token: RwLock<String>,
}

// Manual Debug: never let the cached token through. Deriving would leak the token via
// RwLock<String>'s Debug impl when the lock is currently available.
impl std::fmt::Debug for MyOauth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MyOauth")
            .field("cached_token", &"<redacted>")
            .finish()
    }
}

#[async_trait]
impl AuthProvider for MyOauth {
    async fn authorization_header(&self) -> aviso::Result<HeaderValue> {
        let token = self.cached_token.read().await;
        let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| ClientError::Auth(format!("invalid Bearer header: {e}")))?;
        value.set_sensitive(true);
        Ok(value)
    }

    async fn refresh(&self) -> aviso::Result<()> {
        let new_token = fetch_fresh_token_from_idp().await?;
        *self.cached_token.write().await = new_token;
        Ok(())
    }
}

# async fn fetch_fresh_token_from_idp() -> aviso::Result<String> { Ok("new".into()) }
```

Two non-obvious requirements custom providers must honour:

- **Redact in `Debug`.** Deriving `Debug` over a `RwLock<String>` exposes the wrapped token whenever the lock is uncontended. Implement `Debug` by hand and substitute `"<redacted>"` for the secret, as the shipped `Basic` and `Bearer` do.
- **Map header-construction errors into `ClientError::Auth`.** `HeaderValue::from_str` returns `InvalidHeaderValue`, which does not convert to `ClientError` automatically; the `?` operator would not compile without an explicit `map_err`. Auth is the right variant because a non-ASCII token is fundamentally an auth-configuration problem.

Mark the returned `HeaderValue` as sensitive so the downstream log path redacts it. The shipped providers do this; custom providers must too.
