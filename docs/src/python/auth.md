# Authentication

The Python package includes five auth providers. They wrap the same Rust
providers the CLI uses; pick the one that matches where your credentials live.

Every example on this page constructs a client. Replace the placeholder
credentials with real values for your server (or set the matching environment
variables and switch to `aviso.Env()`).

## Bearer token

<!-- not-runnable -->
```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Bearer("opaque-jwt-or-token"),
)

print(client.schema().event_types)
```

The token is redacted from `repr()` and from every log line the client emits.
Substitute your own token for the placeholder.

## Basic auth

```python
"""Authenticate with username and password."""

import os
import aviso

client = aviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=aviso.Basic(os.environ["AVISO_USERNAME"], os.environ["AVISO_PASSWORD"]),
)

print(client.schema().event_types)
```

The password is redacted from `repr()` and from logs.

## Environment

`aviso.Env()` reads `AVISO_TOKEN`, `AVISO_USERNAME`, and `AVISO_PASSWORD` at
construction. A bearer token wins over the user/password pair when both are set.
If nothing is set, `Env()` raises `aviso.AuthError`.

```python
"""Pick credentials up from environment variables."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

print(client.schema().event_types)
```

This is the most common shape for CI jobs, containers, and one-shot scripts
where credentials live in the environment.

## Config file

<!-- not-runnable -->
```python
import os
import aviso

client = aviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=aviso.ConfigFile("~/.config/aviso/auth.yaml"),
)

print(client.schema().event_types)
```

The file must contain exactly one of:

```yaml
bearer:
  token: opaque-jwt-or-token
```

or:

```yaml
basic:
  username: alice
  password: wonderland
```

Both sections, or neither, is an `aviso.ConfigError`. The path is expanded with
`os.path.expanduser` so `~` works.

## Chain

`Chain` composes providers with first-success-wins semantics. Each member is
tried in order; the first one to produce a credential wins.

<!-- not-runnable -->
```python
import os
import aviso

client = aviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=aviso.Chain(
        aviso.Env(),
        aviso.ConfigFile("~/.config/aviso/auth.yaml"),
        aviso.Bearer("emergency-fallback-token"),
    ),
)

print(client.schema().event_types)
```

If every member fails, the last error propagates as `aviso.AuthError`.

## Refresh on 401

On a 401 the client calls the provider's `refresh()` and retries the original
request once. The five providers shipped today carry static credentials, so
`refresh()` is a no-op and a stale credential still produces a 401 on the retry.
The retry-once contract is in place so a provider that rotates tokens drops in
without a code change; for the shipped providers it is just protocol.

## With `AsyncAvisoClient`

Every provider on this page works identically with the async client. Swap
`AvisoClient` for `AsyncAvisoClient` and the rest of the construction is the
same.
