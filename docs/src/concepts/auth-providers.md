# Authentication providers

aviso supports three authentication modes with the server: anonymous (no
`Authorization` header), HTTP Basic, and Bearer (an opaque or JWT token). Five
built-in providers cover the common ways to supply credentials.

You will rarely pick a provider by name. The CLI builds the right one from your
flags, environment variables, and config file. The names below show up in logs
and in the library API.

## The five providers at a glance

| Provider | Sends | Where the credentials come from |
|---|---|---|
| `Bearer` | `Authorization: Bearer <token>` | Constructor argument |
| `Basic` | `Authorization: Basic <base64>` | Constructor arguments |
| `Env` | Bearer or Basic | `AVISO_TOKEN`, `AVISO_USERNAME`, `AVISO_PASSWORD` |
| `ConfigFile` | Bearer or Basic | A YAML file with a `bearer:` or `basic:` block |
| `Chain` | Whichever member wins first | Composition of any of the above |

All five mark the `Authorization` header as sensitive, so downstream logging
libraries (reqwest, hyper, tracing) redact the value automatically.

## Which one the CLI picks for you

For the CLI:

1. If you pass `--token` (or `AVISO_TOKEN` is set, or the config file has
   `auth.bearer_token`), aviso uses Bearer.
2. If you pass `--username`/`--password` (or `AVISO_USERNAME`/`AVISO_PASSWORD`
   are set, or `auth.basic.{username,password}` is in the config file), aviso
   uses Basic.
3. If both kinds of credentials are present, the flag wins, then the env vars,
   then the file.
4. If no credentials are present, aviso runs anonymously (no `Authorization`
   header). Some servers allow this for public streams.

The CLI never asks you which provider to use. You set the values; aviso picks
the provider.

## When you need `ConfigFile`

You have multiple environments (staging, prod) with separate credentials, and
you do not want them in the main config file. Store the credentials in a
separate YAML and point at it programmatically (from a Rust caller) with
`ConfigFile::from_path`.

The CLI uses the layered config (file + env + flags), not `ConfigFile` directly.
You will only call `ConfigFile::from_path` from a Rust program that does its own
composition.

The file shape:

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

Only one of `bearer:` and `basic:` per file. The parser rejects both-or-neither
and unknown keys, so a typo fails loudly.

## When you need `Chain`

You want aviso to fall back from one credential source to another. For example,
prefer a short-lived token from the environment, fall back to a long-lived
service-account token in a config file.

In a Rust program:

```rust,ignore
use std::sync::Arc;
use aviso::auth::{AuthProvider, Chain, ConfigFile, Env};

let mut providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
if let Ok(env) = Env::from_process_env() {
    providers.push(Arc::new(env));
}
if let Ok(file) = ConfigFile::from_path("/etc/aviso/fallback.yaml") {
    providers.push(Arc::new(file));
}
let chain = Chain::new(providers);
```

`Chain` tries each member in order. The first one whose `authorization_header()`
returns `Ok` wins. The CLI does not expose `Chain` directly because its own
layered config already covers the common case.

## What happens on a 401

If the server returns `401 Unauthorized`, aviso asks the auth provider to
refresh and retries the request once.

- For `Bearer` and `Basic`, refresh is a no-op. A second 401 means the
  credential really is wrong.
- For `Env` and `ConfigFile`, refresh re-reads the source. If you rotated the
  env var or the file between requests, the new value is picked up.
- For `Chain`, refresh targets the member whose header went out on the failing
  request, not all members.

For custom providers (an OAuth client, an OIDC client, a signed-URL generator),
you implement `refresh()` to rotate the cached credential. The next request
picks up the new value.

A second 401 in the same attempt cycle terminates the request with an
authentication error.

## Writing a custom provider

If you need something the five built-in providers do not cover (OAuth, OIDC, AWS
SigV4), implement the `AuthProvider` trait in your own code:

```rust,ignore
use async_trait::async_trait;
use aviso::{auth::AuthProvider, ClientError};
use reqwest::header::HeaderValue;
use tokio::sync::RwLock;

struct MyOauth {
    cached_token: RwLock<String>,
}

#[async_trait]
impl AuthProvider for MyOauth {
    async fn authorization_header(&self) -> aviso::Result<HeaderValue> {
        let token = self.cached_token.read().await;
        let mut v = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| ClientError::Auth(format!("invalid header: {e}")))?;
        v.set_sensitive(true);
        Ok(v)
    }

    async fn refresh(&self) -> aviso::Result<()> {
        let new_token = fetch_from_idp().await?;
        *self.cached_token.write().await = new_token;
        Ok(())
    }
}
# async fn fetch_from_idp() -> aviso::Result<String> { Ok("new".into()) }
```

Two things to remember:

- Implement `Debug` by hand and redact the secret. The default derivation can
  leak through the `RwLock`'s `Debug`.
- Always call `HeaderValue::set_sensitive(true)` on the value you return. That
  is what makes downstream loggers redact it.

## What next

- [CLI configuration: authentication](../cli/configuration.md#authentication):
  the layered flag-env-file resolution.
- [Library guide](../developers/lib-guide.md): how to compose providers
  programmatically.
