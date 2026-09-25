<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Configuration

A client needs a server address and, usually, a credential. Both can be
passed in code. On a machine where the `aviso` command is already configured,
they are defined once, in the environment or in `~/.config/aviso/config.yaml`,
and scripts need not repeat them. Every constructor argument is therefore
optional, and any argument that is omitted is looked up:

```python
import pyaviso

client = pyaviso.AvisoClient()
print(client.schema().event_types)
```

This page describes where each setting is read from, in which order, and how
to inspect the configuration a client resolved.

## Order of precedence

For each setting, the client uses the first source that provides a value:

| Setting | 1. Code | 2. Environment | 3. Config file | 4. Otherwise |
|---|---|---|---|---|
| `base_url` | `base_url=` | `AVISO_BASE_URL` | `base_url:` | `ConfigError` |
| `auth` | `auth=` | `AVISO_TOKEN`, or `AVISO_USERNAME` with `AVISO_PASSWORD` | `auth:` block, then the credentials file | anonymous |
| `timeout` | `timeout=` | | `timeout:` | no timeout |
| `heartbeat_interval` | `heartbeat_interval=` | | `heartbeat_interval:` | 30 seconds |
| TLS | `danger_accept_invalid_certs=` | | `tls:` block | validate certificates |

The config file is `~/.config/aviso/config.yaml`, or the file named by
`AVISO_CLIENT_CONFIG_FILE`. The credentials file is
`~/.config/aviso/credentials.yaml`, or `AVISO_CREDENTIALS_FILE`. A missing
file sets nothing; a file that exists but cannot be read raises
`pyaviso.ConfigError`. Sections intended for the `aviso` command, such as
`listeners`, are ignored.

The `aviso` command uses the same order, with its command-line options taking
precedence, so a script and the command on the same machine use the same
server and the same credential. Two consequences follow:

- **The environment can change where a script connects.** An `AVISO_BASE_URL`
  set in the shell determines where `AvisoClient()` connects. A script that
  must always use one server should pass `base_url=`.
- **A discovered credential is protected.** A credential read from the
  environment or a file is not sent to a plain `http://` address unless the
  host is loopback; the client raises `pyaviso.AuthError` instead. Because
  the credential was not named in code, a mistyped host would otherwise
  receive it unencrypted. Passing `auth=` explicitly removes this
  restriction. See [Authentication](./auth.md) for details.

`AvisoClient.from_file(path)` is the same lookup with a named config file in
place of the default one; that path must exist. `AsyncAvisoClient` takes the
same arguments and follows the same order.

## Inspecting the resolved configuration

When a connection fails, the first question is which server and which
credential the client is using. Every client reports this:

```python
import pyaviso

client = pyaviso.AvisoClient()
print(client.config)
```

```text
ResolvedConfig(
    base_url='https://aviso.example.org/'            (environment AVISO_BASE_URL),
    auth="bearer"                                    (credentials file /home/me/.config/aviso/credentials.yaml),
    timeout=30.0                                     (config file /home/me/.config/aviso/config.yaml),
    heartbeat_interval=None                          (default),
    ca_bundle=[]                                     (default),
    danger_accept_invalid_certs=False                (default),
)
```

Each line shows a setting, its value and its source. The report contains no
secrets: the credential is described by its kind and source, never by its
value, and any `user:password@` is removed from the address. The report can
therefore be included as it is in a support request or a log.

The fields are also available as objects:

```python
url = client.config.base_url
if url is not None:
    print(url.value, "from", url.source)

if client.config.auth is None:
    print("no credential found: requests are anonymous")
```

`source` is one of `code`, `environment <NAME>`, `config file <path>`,
`credentials file <path>` or `default`. `auth` is `None` when no source had a
credential; `auth=pyaviso.Anonymous()` is instead reported as kind
`anonymous` from `code`, since that was an explicit choice.
`client.config.as_dict()` returns the same information as plain values, for
structured logging.

## Inspecting the configuration without a client

`pyaviso.resolve_config()` produces the same report without creating a
client. It takes the constructor arguments that take part in the lookup
(`base_url`, `auth`, `timeout`, `heartbeat_interval` and
`danger_accept_invalid_certs`; not `user_agent`, `state_store` or
`flush_cursor_on_exit`, which are never looked up) and works even when a
client could not be built:

```python
import pyaviso

config = pyaviso.resolve_config()
if config.base_url is None:
    print("no server address in code, AVISO_BASE_URL or the config file")
elif config.auth is not None and config.auth.refused:
    print("credential found but not sent:", config.auth.refused)
else:
    print(config)
```

The second case is a common source of confusion: a credential is configured,
yet `AvisoClient()` raises `AuthError`. The report shows the credential and
the reason the client refuses to use it, for example:

```text
    auth="bearer"                                    (config file /home/me/.config/aviso/config.yaml; refused: http://aviso.internal.example.org is plain http on a host that is not loopback, so the client will not be built. Use https, or name the credential in code to send it anyway.),
```

`resolve_config()` raises `pyaviso.ConfigError` when the config file or the
credentials file exists but cannot be read, and `pyaviso.AuthError` when a
credential source is present but unusable, such as `AVISO_USERNAME` with no
`AVISO_PASSWORD`; that is the same point at which `AvisoClient()` would
fail. All other conditions are reported in the returned object.

## Setting values in code

Any value passed to the constructor is used as given and reported with the
source `code`:

```python
client = pyaviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=pyaviso.Bearer(token),
    timeout=10,
)
```

Here the address and credential are fixed regardless of the machine's
configuration, while `heartbeat_interval` and the TLS settings are still read
from the file if it sets them. To send no credential even when one is
configured on the machine, pass `auth=pyaviso.Anonymous()`.

## The `aviso` command

`aviso config dump` prints the same kind of report for the command, including
the command-line flags the library does not have. See
[CLI configuration](../cli/configuration.md).
