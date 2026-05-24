# Authentication

Five auth providers ship in the Python package. They wrap the same Rust providers the CLI uses; pick the one that matches where your credentials live.

## Bearer token

```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Bearer("opaque-jwt-or-token"),
)
```

The token is redacted from `repr()` and from every log emitted by the client.

## Basic auth

```python
client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Basic("alice", "wonderland"),
)
```

The password is redacted from `repr()` and from logs.

## Environment

```python
client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Env(),
)
```

`Env()` reads `AVISO_TOKEN`, `AVISO_USERNAME`, and `AVISO_PASSWORD` at construction. A bearer token wins over the user/password pair when both are set. If nothing is set, `Env()` raises `aviso.AuthError`.

## Config file

```python
client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.ConfigFile("~/.config/aviso/auth.yaml"),
)
```

The file must contain exactly one of:

```yaml
bearer:
  token: opaque-jwt-or-token
```

or

```yaml
basic:
  username: alice
  password: wonderland
```

Both sections, or neither, is an `aviso.ConfigError`.

## Chain

`Chain` lets you compose providers with first-success-wins semantics:

```python
client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Chain(
        aviso.Env(),
        aviso.ConfigFile("~/.config/aviso/auth.yaml"),
        aviso.Bearer("emergency-fallback-token"),
    ),
)
```

The chain tries each member in order. If all members fail, the last error propagates as `aviso.AuthError`.

## Refresh

The chosen provider's `refresh()` method runs automatically on a 401, with one retry. The shipped Bearer and Basic providers have no refresh logic (the 401 propagates immediately); a custom Rust provider with token-rotation behaviour rotates on 401 transparently when added.
