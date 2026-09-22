<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Configuration

How aviso decides what server to talk to, how to authenticate, what TLS settings
to use, and where to keep state.

## Tell aviso where the server is

The minimum aviso needs is a server URL and (usually) credentials. You have
three ways to supply each one. They are checked in this order:

1. Command-line flag (`--base-url`, `--token`, ...).
2. Environment variable (`AVISO_BASE_URL`, `AVISO_TOKEN`, ...).
3. Config file (default `~/.config/aviso/config.yaml`).

A flag beats an env var beats a config file. Layering is per-field: passing
`--base-url` does not blank out a file-set `auth.bearer_token`.

To see what aviso actually resolved, run `aviso config dump --redact`.

The base URL must use `http` or `https` and point to the Aviso service,
including any reverse-proxy path prefix. A website or login page is not a
stream endpoint.

## Listener startup timeout

`aviso listen` prints `Connecting` first. It prints `Listening` for each
listener only after the server confirms an Aviso stream. No matching
notification is needed: a confirmed stream can be healthy and idle.

`--startup-timeout <DURATION>` limits the initial connection and retries to 30
seconds by default. This gives transient failures time to recover while keeping
a failed startup bounded. To change that budget for configured listeners:

```bash
aviso listen --startup-timeout 60s
```

Use `--startup-timeout 0s` to disable the initial budget. This listen-only flag
has no YAML key or environment variable. It stops applying after the first
confirmed handshake and does not restart on reconnect. Each connection attempt
still has a separate ten-second deadline for response headers and the Aviso
opening event. Heartbeats and unrelated SSE events do not confirm startup.

Retry status goes to stderr at INFO level, with a listener name, cause, and
delay. Repeated retries are coalesced to at most one message every five seconds.
Notifications keep their configured trigger output. Ctrl+C while connecting
stops the listener normally. A failed listener makes the eventual command exit
with status 1; other listeners keep running.

## The configuration file

```yaml
# ~/.config/aviso/config.yaml

base_url: "https://aviso.example"

auth:
  bearer_token: "your-token-here"
  # Or use basic auth instead:
  # basic:
  #   username: "alice"
  #   password: "secret"

# Optional, with sensible defaults if omitted:
heartbeat_interval: 30s
state_file: "/path/to/state.json"

tls:
  ca_bundle:
    - "/path/to/internal-ca.pem"
  danger_accept_invalid_certs: false

listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
    triggers:
      - type: log
        path: /var/log/aviso/mars-od.log
```

To point at a config file in another location, use `--config <PATH>` or set
`AVISO_CLIENT_CONFIG_FILE`.

The optional YAML `timeout` is the total HTTP request timeout, including a
stream's body. It is unset by default. Leave it unset for long-lived listeners;
use `--startup-timeout` to bound startup without limiting a healthy stream.

## Environment variables

| Variable | What it sets |
|---|---|
| `AVISO_BASE_URL` | The server URL. |
| `AVISO_TOKEN` | A bearer token. |
| `AVISO_USERNAME` / `AVISO_PASSWORD` | Basic auth credentials. |
| `AVISO_CLIENT_CONFIG_FILE` | Path to the config file. |
| `AVISO_CREDENTIALS_FILE` | Path to the credentials file. |
| `AVISO_STATE_FILE` | Path to the state file. |
| `AVISO_LOG` | Logging filter. When set, overrides `-v`/`-vv`. Format: a [`tracing_subscriber`](https://docs.rs/tracing-subscriber) `EnvFilter` directive. |
| `NO_COLOR` | When set (any value), suppresses ANSI colors in the `--color auto` mode. Per the [no-color.org](https://no-color.org/) convention. |

## Authentication

aviso supports anonymous access (no `Authorization` header), HTTP Basic, and
Bearer. It looks for a credential in four places and stops at the first one
that has it:

| Order | Source |
|---|---|
| 1 | The `--token` flag, or `--username` with `--password`. |
| 2 | The `AVISO_TOKEN` env var, or `AVISO_USERNAME` with `AVISO_PASSWORD`. |
| 3 | `auth.bearer_token`, or `auth.basic.{username,password}`, in the config file. |
| 4 | The credentials file described below. |

The flags are the least safe of the four. A command line is visible to every
local user in the process list and is kept in the shell history, so
`--token` and `--password` belong to one-off tests, not to scripts or service
definitions. `aviso` logs one WARN line, `cli.auth.on_command_line`, when a
credential arrives this way. Use the environment variables or the credentials
file instead.

Once an earlier source supplies a credential, the `auth:` block of the config
file and the credentials file are not interpreted, so a stale entry in either
cannot fail a command that was not going to use it. The config file itself is
still parsed for its other settings, and a YAML error there is reported
regardless. With nothing in any of the four, aviso connects anonymously.

The winning source decides the provider: the environment builds an `Env`, the
credentials file a `ConfigFile`, and a flag or the config-file `auth:` block a
`Bearer` or a `Basic` depending on which credential you set. Those names
appear in logs and in the library API; see
[Authentication providers](../concepts/auth-providers.md) for what each one
does.

### The credentials file

When nothing else supplies a credential, aviso reads
`~/.config/aviso/credentials.yaml`. Set `AVISO_CREDENTIALS_FILE` to use a
different path. The file holds one credential and nothing else:

```yaml
# ~/.config/aviso/credentials.yaml
bearer:
  token: your-bearer-token
```

Or, for Basic authentication:

```yaml
basic:
  username: your-username
  password: your-password
```

This file is meant for tools that fetch a token and write it out. It is read
last, so a credential you set with a flag, in the environment, or in the `auth:`
block of the config file is used instead. A missing file is fine. A file that
exists but cannot be read is an error, so a typo is reported rather than
ignored.

The credentials file is also the only source that is reread after a 401. A
tool that refreshes the token in place therefore reaches a running
`aviso listen` without a restart.

A credential from the environment or either file is not sent to a plain
`http://` address unless it is loopback. Use an `https` address, or pass the
credential on the command line with `--token`, or `--username` and
`--password`, to say you mean it. This applies when a command makes a request;
`config dump` still reports the source it would have refused. A credential
you named that goes to a non-loopback `http://` address is sent, with one
WARN line, `client.auth.plaintext`, the same way `--danger-accept-invalid-certs`
is announced.

To see which source is in use:

```bash
aviso config dump | grep -A2 '^auth:'
```

### On a 401, aviso retries once

If the server returns `401 Unauthorized`, aviso asks the auth provider to
refresh and retries the request once. For static credentials (bearer, basic) the
refresh is a no-op; for providers backed by an OAuth or OIDC cache, the refresh
rotates the token. A second 401 in the same attempt cycle is surfaced as an
error.

## The state file

Each successful listener run records its cursor in `~/.config/aviso/state.json`.
On restart, the listener resumes from the next sequence so it does not skip
ahead. A notification can still be redelivered after a crash or failed
checkpoint.

Change the location:

```yaml
state_file: "/var/lib/aviso/state.json"
```

or:

```bash
aviso --state-file /var/lib/aviso/state.json listen ...
```

Disable persistence for one run:

```bash
aviso listen --no-state-store ...
```

The file is for `aviso listen` only. `aviso replay` does not touch it.

For the file format, edit safety, and how to genuinely reset a cursor, see
[State file](../reference/state-file.md).

## `--from` value formats {#from-value-formats}

Both `aviso replay --from <VALUE>` (required) and `aviso listen --from <VALUE>`
(optional, overrides the listener's defaults) accept these forms, tried in
order:

1. **Pure digits** → sequence id. `42`, `1234567`.
2. `YYYY-MM-DD` → midnight UTC. `2026-05-01`.
3. `"YYYY-MM-DD HH:MM"` (quotes required) → UTC. `"2026-05-01 14:30"`.
4. `"YYYY-MM-DD HH:MM:SS"` (quotes required) → UTC.
5. `YYYY-MM-DDTHH:MM:SS` (T separator) → UTC.
6. `YYYY-MM-DDTHH:MM:SSZ` (T separator with Z) → UTC.
7. `YYYY-MM-DDTHH:MM:SS.ffffffZ` (with microseconds and Z) → UTC.

**The pure-digit case is always a sequence id**, never a date. `20260601` is
sequence id 20260601, not 1 June 2026. To pass a date, use the dashed form
(`2026-06-01`).

### How `--from` interacts with the state file

On `aviso listen`, when you pass `--from <VALUE>` *and* the state file already
has a cursor for the listener:

- aviso uses your `--from` for the initial seek.
- As the rewind delivers notifications, the state file ignores updates whose
  sequence is at or below what is already on disk. Your `--from` cannot regress
  the cursor.
- Once the run advances past the previous high-water mark, normal updates
  resume.
- Restarting without `--from` honours the state file again.

Use `--from` as a one-shot rewind. Leaving it in a systemd unit means redelivery
from that point on every restart.

## Logging verbosity

| | Effect |
|---|---|
| Default | `aviso` crate at INFO, third-party crates at WARN. |
| `-v` | `aviso` crate at DEBUG. |
| `-vv` | `aviso` crate at TRACE. |
| `AVISO_LOG=<directive>` | Overrides everything else. Operator policy wins. |

Useful recipes for the env var:

```bash
AVISO_LOG=warn,aviso=debug              # aviso details, everything else quiet
AVISO_LOG=h2=debug,hyper=debug,aviso=debug  # also see HTTP transport
```

## Color

`--color auto|always|never`. The default is `never`. The `auto` mode emits color
when the target stream is a TTY and `NO_COLOR` is unset. `always` overrides
`NO_COLOR`.

Color only ever applies to human-readable output. JSON output (piped or
redirected) is always plain.

## TLS

aviso talks HTTPS by default and uses the system trust store. Two flags adjust
validation when that is not enough.

### Trust an internal CA

When your aviso-server is fronted by a TLS endpoint whose certificate is signed
by an internal certificate authority (a corporate root, a self-hosted ACME, a
private cluster), point aviso at the CA file:

```bash
aviso --base-url https://aviso.internal.example.org --ca-bundle ~/.config/aviso/internal-ca.pem schema list
```

Or in the config file:

```yaml
tls:
  ca_bundle:
    - internal-ca.pem
```

A relative path here is resolved against the config file's own directory, so
this entry means `~/.config/aviso/internal-ca.pem` wherever you run `aviso`
from. A relative `--ca-bundle` on the command line is resolved against the
working directory, as you would expect of a flag.

The `--ca-bundle` flag is repeatable, so you can pass intermediate and root
certificates separately (or put them all in one PEM file). The system trust
store stays in effect; `--ca-bundle` only adds.

To fetch the CA's certificate from a running server (for inspection or for use
here):

```bash
openssl s_client -connect aviso.internal.example.org:443 -showcerts < /dev/null 2>/dev/null \
  | sed -n '/-----BEGIN CERTIFICATE-----/,/-----END CERTIFICATE-----/p' \
  > internal-ca.pem

openssl x509 -in internal-ca.pem -noout -subject -issuer
```

### Bypass TLS validation (insecure)

For short-lived development against a self-signed certificate when shipping the
CA file is impractical:

```bash
aviso --base-url https://localhost:8443 --danger-accept-invalid-certs schema list
```

aviso logs a `WARN` at every startup when this is set, so log scrapers can flag
misuse. Do not use this in production.

The recommended order: try the system trust store first, fall back to
`--ca-bundle`, use `--danger-accept-invalid-certs` only as a last resort during
development.

## What next

- [Listener YAML reference](../reference/listener-yaml.md): the file format the
  CLI reads.
- [State file reference](../reference/state-file.md): annotated example, edit
  safety.
- [Authentication providers](../concepts/auth-providers.md): when to use which
  one.
