<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Configuration

A client needs a server address, and usually a credential. You can pass both
in code, but on a machine that already runs the `aviso` command they are set
up once, in the environment or in `~/.config/aviso/config.yaml`, and a script
should not have to repeat them. So every constructor argument is optional, and
what you leave out is looked up:

```python
import pyaviso

client = pyaviso.AvisoClient()
print(client.schema().event_types)
```

This page says where each setting comes from, in what order, and how to see
what a client ended up with.

## Where settings come from

For each setting the client takes the first source that has a value:

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
`pyaviso.ConfigError`. Sections the file has for the `aviso` command, such as
`listeners`, are ignored here.

This is the same order the `aviso` command uses, minus its command-line flags,
so a script and the command on the same machine talk to the same server with
the same credential. Two consequences follow:

- **The environment can redirect a script.** An `AVISO_BASE_URL` left in the
  shell changes where `AvisoClient()` connects. If a script must always talk
  to one server, pass `base_url=`.
- **A found credential is protected.** A credential from the environment or a
  file is not sent to a plain `http://` address unless it is loopback; the
  client raises `pyaviso.AuthError` instead. You did not name the credential
  in code, so a mistyped host would otherwise send it in the clear. Passing
  `auth=` yourself lifts the rule. [Authentication](./auth.md) has the
  details.

`AvisoClient.from_file(path)` is the same lookup with a named config file in
place of the default one; that path must exist. `AsyncAvisoClient` takes the
same arguments and follows the same order.

## See what a client resolved

When something does not connect, the first question is which server and which
credential the client ended up with. Every client answers it:

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

Each line is a setting, its value, and where it came from. Nothing in it is a
secret: the credential is described by its kind and source, never its value,
and the address has any `user:password@` removed. The whole thing is written
to be pasted into a ticket or a log.

The fields are objects too, so code can read them:

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
`anonymous` from `code`, since that was a choice. `client.config.as_dict()`
returns the same information as plain values, for a JSON log line.

## Ask before building

`pyaviso.resolve_config()` produces the same report without building a
client. It takes the constructor's arguments and works even when a client
could not be built:

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

The second case is the one that confuses people most: a token is set up, and
`AvisoClient()` raises `AuthError` anyway. The report shows the credential
and says why the client refuses to build with it, for example:

```text
    auth="bearer"                                    (config file /home/me/.config/aviso/config.yaml; refused: http://aviso.internal.example.org is plain http on a host that is not loopback, so the client will not be built. Use https, or name the credential in code to send it anyway.),
```

`resolve_config()` raises `pyaviso.ConfigError` when the config file or the
credentials file exists but cannot be read, and `pyaviso.AuthError` when a
credential source is present but unusable, such as `AVISO_USERNAME` with no
`AVISO_PASSWORD`; that is the same point at which `AvisoClient()` would
fail. Everything else is reported in the object.

## Pin a setting in code

Anything passed to the constructor is taken as given and reported with the
source `code`:

```python
client = pyaviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=pyaviso.Bearer(token),
    timeout=10,
)
```

Here the address and credential are fixed whatever the machine has set up,
while `heartbeat_interval` and the TLS settings still come from the file if
it has them. To send no credential even though one is present on the machine,
pass `auth=pyaviso.Anonymous()`.

## The `aviso` command

`aviso config dump` prints the same kind of report for the command, including
the command-line flags the library does not have. See
[CLI configuration](../cli/configuration.md).
