# Command-line interface

The `aviso` binary is a thin command-line client over the [`aviso`](../internals/architecture.md) library. It publishes notifications, runs listeners, replays history, inspects the server's schema, and runs destructive admin operations. The implementation lives at `crates/aviso-cli/`.

## Installation

```bash
cargo install aviso-cli
```

The published binary is named `aviso` (set via `[[bin]] name = "aviso"` in the crate manifest). Run `aviso --version` to confirm the install.

For development against an unreleased revision, clone the repo and run `cargo install --path crates/aviso-cli`.

## Quick start

Three workflows cover the bulk of operator usage.

### Publish a notification

```bash
aviso \
  --base-url https://aviso.example.org \
  --token "$AVISO_TOKEN" \
  notify 'event=mars,class=od,stream=oper,date=20260521,domain=g,expver=0001,step=0,time=1200,data={"region":"north"}'
```

The single positional argument is a comma-separated `key=value` list (pyaviso parity). Only `event=<TYPE>` is mandatory at the parameter-parsing layer; the server's notify endpoint additionally requires every identifier key listed in the event-type's schema. The schema's `required: false` flag is a **`listen`/`replay`-time filter** semantic (when subscribing, optional identifiers act as wildcards); it does **not** make those identifiers optional for `notify`. Use `aviso schema get <TYPE>` to see the full identifier set the server will demand.

The optional `data=<JSON>` key becomes the notification payload; every other `key=value` pair enters the identifier map.

**Quoting identifier values containing commas**: for identifiers whose values contain top-level commas (`polygon`, lists of identifiers, etc.), wrap the value in double quotes. The CLI's parameter splitter treats top-level commas as parameter separators, but inside `"..."` they are part of the value; the outer quotes are stripped before the value is sent to the server (pyaviso convention):

```bash
aviso notify 'event=test_polygon,polygon="46,8,46,9,47,9,47,8,46,8",date=20260521,time=1200,data={"test":true}'
```

### Run a listener

```bash
aviso \
  --base-url https://aviso.example.org \
  --token "$AVISO_TOKEN" \
  listen my_listeners.yaml
```

Where `my_listeners.yaml` is:

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
      stream: oper
    triggers:
      - type: echo
      - type: webhook
        url: "https://hooks.example.org/notify"
        headers:
          Authorization: "Bearer {{ env.HOOK_TOKEN }}"
```

The CLI spawns one task per listener; the trigger pipeline handles output. The CLI itself prints nothing to stdout on the `listen` subcommand.

### Inspect the resolved configuration

```bash
aviso --config ~/.config/aviso/config.yaml config dump --redact
```

The dump shows the resolved configuration with `# from: flag|env|file|default` source-attribution comments on the layered scalar and list fields (`base_url`, `timeout`, `heartbeat_interval`, `tls.ca_bundle`, `tls.danger_accept_invalid_certs`, `config_path`, `state_file`). The `auth` block is summarised as `provider: <set>`, `<set; redacted>`, or `<unset>` rather than per-field source-tagged: the auth chain composes flag, env, and file layers into a single `Arc<dyn AuthProvider>` and does not surface a single winning source, and even un-redacted output should not name which secret store supplied the live credential. The `listeners` block is summarised by name, event, identifier count, and trigger count. `--redact` masks tokens and passwords. `--json` forces JSON output (the source tags become `source: ...` fields).

## Subcommand reference

| Subcommand | Purpose | Output discipline |
|---|---|---|
| `aviso notify <PARAMS>` | POST one notification | TTY: human line; pipe / `--json`: NDJSON with `NotifyResponse` |
| `aviso listen [FILES...]` | Run one or more listeners concurrently. With `--event <TYPE> --identifiers <JSON>` runs a single ad-hoc listener without a YAML file (default echo trigger; same `--from` and state-store semantics as YAML mode). See [Listening without a YAML file (inline mode)](#listening-without-a-yaml-file-inline-mode) | The CLI itself emits no stdout; configured triggers do. `echo` (the inline-mode default) writes NDJSON on pipe / pretty JSON on TTY. `log` writes to a file. Other triggers reach their own sinks |
| `aviso replay --from <VALUE> [FILES...]` | Replay historical notifications from a cursor (positional listener YAMLs, `--listener <NAME>` to pick one from the resolved set, or `--event <TYPE> --identifiers <JSON>` for ad-hoc). Stateless: never reads or writes the state file (see [`aviso replay` and the state file](../resume/state-file.md#aviso-replay-and-the-state-file)) | Same as `aviso listen`: the CLI emits no stdout; triggers do. Ad-hoc replay defaults to a single `echo` trigger for the same pipe-friendly behavior |
| `aviso schema list` | GET `/api/v1/schema` (index of registered event-type names) | TTY: bullet list; pipe / `--json`: NDJSON. Same content, different rendering. Use `aviso schema get <TYPE>` for the full schema of one entry, or `aviso schema list \| xargs -I{} aviso schema get {}` for all. |
| `aviso schema get <TYPE>` | GET single schema | Pretty-printed JSON (always) |
| `aviso admin wipe-stream <EVENT> --yes` | DELETE one event-type stream | TTY: `ok:` line; `--json`: NDJSON |
| `aviso admin wipe-all --yes` | DELETE every notification | TTY: `ok:` line; `--json`: NDJSON |
| `aviso admin delete <ID> --yes` | DELETE one notification by id | TTY: `ok:` line; `--json`: NDJSON |
| `aviso config dump [--redact]` | Show resolved config | YAML (default) or JSON (`--json`) |
| `aviso completions <SHELL>` | Print shell completions | Shell-specific script to stdout |

Run `aviso <SUBCOMMAND> --help` for per-command flags.

## Configuration file

The default config path is `~/.config/aviso/config.yaml` on every platform. Override with `--config <PATH>` or the `AVISO_CLIENT_CONFIG_FILE` env var.

```yaml
base_url: "https://aviso.example.org"
auth:
  bearer_token: "..."          # OR
  basic:
    username: "alice"
    password: "..."
timeout: 30s                    # optional
heartbeat_interval: 30s         # optional
state_file: "/path/to/state.json"  # optional; default ~/.config/aviso/state.json (see docs: Resume & state / The state file)
tls:
  ca_bundle: ["/path/to/ca.pem"]
  danger_accept_invalid_certs: false

listeners:
  - name: mars-od-fc
    event: mars
    identifiers:
      class: od
      stream: oper
      type: fc
    triggers:
      - type: log
        path: /var/log/aviso/mars-od.log
      - type: webhook
        url: "https://hooks.example.org/notify"
        headers:
          Authorization: "Bearer {{ env.HOOK_TOKEN }}"
```

### Per-field precedence

Every field is resolved in this order: `flag > env > file > default`. Layering is per-field, not whole-config replacement: passing `--base-url <URL>` does not blank out a file-set `auth.bearer_token`. `aviso config dump` shows the final resolved values plus the source of each.

### Listener YAML shape

The listeners list is pyaviso-compatible with one rename: pyaviso's `request:` is `identifiers:` here (matching the lib's `Notification::identifier` field). Trigger configs reuse the library's `TriggerConfig` deserialiser so all six shipped trigger kinds (`echo`, `log`, `command` on Unix, `webhook`, `teams`, `post`) work from YAML unchanged.

### Positional listener files

`aviso listen file1.yaml file2.yaml ...` accepts a variadic positional list of listener YAML files. Each carries its own top-level `listeners:` list. Positional files **REPLACE** (not merge with) the global config's `listeners:` for the invocation. With no positional files and no global `listeners:`, the CLI exits with code `2` and a stderr message naming both paths checked.

### Listening without a YAML file (inline mode)

For quick shell-driven peeks at a stream you can skip the YAML file entirely:

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

This runs a single ad-hoc listener with a default `echo` trigger. On a TTY (interactive shell) the echo trigger emits a multi-line pretty-printed JSON block per notification preceded by a `new notification (listener: ad-hoc, trigger: echo):` leader; when stdout is a pipe or redirected to a file (`> out.ndjson`, `| jq`) it emits one compact NDJSON line per notification, the same shape `aviso notify` produces. Pipe to `jq` (or any line-based tool) the same way:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' | jq -r '.payload'
```

#### When to use inline vs YAML

| Concern | Inline (`--event`/`--identifiers`) | YAML (`listeners:` block) |
|---|---|---|
| Quick exploration, ad-hoc piping to a shell tool | Best fit | Overkill (file just to print events) |
| Multiple listeners concurrent | Not supported (inline is single-listener) | Use YAML; supervisor spawns each in its own task |
| Triggers other than `echo` (`log`, `command`, `webhook`, `teams`, `post`) | Not supported (inline is echo-only) | Required |
| Per-listener `from_id` / `from_date` defaults | Use `--from` (optional; defaults to state-file cursor or "now") | YAML can carry the default |
| Production deployment, long-running service | Discouraged (single hard-coded `echo` trigger, generic `ad-hoc` listener name → not auditable, no operator-attached identity) | Recommended (auditable config, named listeners, full trigger surface) |

#### Flag pairing

`--event` and `--identifiers` are a required pair: passing one without the other exits `2` with a clap-generated message naming the missing flag. The `--identifiers` value must be a JSON object literal; the canonical shape is `'{"key":"value", ...}'`. An empty object `{}` is a valid wildcard listener (every notification for the given event type, regardless of identifier).

#### Precedence with positional YAML files

When both inline flags AND a positional YAML file are supplied, **inline wins**. The YAML file is silently ignored on that invocation. This matches `aviso replay`'s existing behavior so operators using either subcommand follow one mental model. The startup banner names `ad-hoc` (not the YAML's listener names) when inline mode takes precedence, so it is always visible which path resolved.

#### State store and resumability

Inline mode is still `aviso listen`: the supervisor checkpoints to the state file with the at-least-once delivery contract intact. Two inline invocations with the same `--base-url` + `--event` + `--identifiers` share a [`ResumeKey`](../resume/overview.md#the-key-resumekey), so the second run resumes from where the first left off. Pass `--no-state-store` for throwaway runs where the cursor should not persist (the in-memory store is dropped on exit; the next run starts from `--from`, or from "now" if `--from` is absent).

#### Cursor

`--from <VALUE>` accepts the same seven forms documented under [`--from` value formats](#from-value-formats) and applies to the inline listener exactly as it would to a YAML one. `--from` is **optional** on listen (default: resume from the state file if available, otherwise start at "now") and **mandatory** on `aviso replay --event ... --identifiers ...`.

#### Custom triggers

To use any trigger other than `echo` (or to mix multiple triggers) on an ad-hoc subscription, write a YAML file and use that instead. The YAML adds about five lines and unlocks the full trigger surface:

```yaml
# inline-with-log.yaml
listeners:
  - name: mars-od-peek
    event: mars
    identifiers:
      class: od
    triggers:
      - type: echo
      - type: log
        path: /tmp/mars-od-peek.log
```

```bash
aviso listen inline-with-log.yaml
```

### `--color auto|always|never`

Global flag controlling ANSI color escapes in the human-readable output paths. The flag REQUIRES a value (`--color auto`, `--color always`, or `--color never`); bare `--color` would be ambiguous with subcommand parsing and is rejected.

| Mode | stderr (tracing events) | stdout (echo trigger human form) | JSON paths (pipe/file) |
|---|---|---|---|
| `never` (default) | no color | no color | no color |
| `auto` | colored when stderr is a TTY and `NO_COLOR` is unset | colored when stdout is a TTY and `NO_COLOR` is unset | no color |
| `always` | colored regardless of TTY; overrides `NO_COLOR` | colored regardless of TTY; overrides `NO_COLOR` | no color |

Per-stream: stderr (tracing) and stdout (echo trigger) are evaluated independently so `aviso listen --color auto | jq` correctly keeps stderr colored (TTY) and stdout JSON (pipe). ANSI is never emitted into JSON forms regardless of the flag value (`auto`/`always` only affect human-readable output; pipe/file consumers always get clean JSON).

## Environment variables

| Variable | Effect |
|---|---|
| `AVISO_LOG` | `tracing_subscriber` `EnvFilter` directive. When set, **overrides** `-v`/`-vv` and is the authoritative logging policy. Without it: the `aviso` crates honour `-v` (INFO/DEBUG/TRACE) while every other crate stays at WARN. Useful recipes: `AVISO_LOG=warn,aviso=debug` for app-only DEBUG, `AVISO_LOG=h2=debug,hyper=debug,aviso=debug` for transport-level diagnostics. |
| `NO_COLOR` | When set (any value), suppresses ANSI color escapes in the `--color auto` mode. `--color always` explicitly overrides `NO_COLOR` per the convention at <https://no-color.org/>. `--color never` (and the default) are unaffected. |
| `AVISO_CLIENT_CONFIG_FILE` | Config file path. Lower priority than `--config`. |
| `AVISO_STATE_FILE` | State file path. Lower priority than `--state-file`. |
| `AVISO_BASE_URL` | Base URL. Lower priority than `--base-url`. |
| `AVISO_TOKEN` | Bearer token. Lower priority than `--token`. |
| `AVISO_USERNAME` / `AVISO_PASSWORD` | Basic auth credentials. Lower priority than the flags. |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. A clean Ctrl+C with no prior listener failure also returns 0. |
| `1` | Runtime error: server returned 4xx / 5xx, network failure, file I/O failure, or one or more `aviso listen` listener tasks errored / panicked. |
| `2` | Usage error: missing required flag, invalid argument value, destructive admin command without `--yes`, no listeners resolved for `aviso listen`, unparseable `--from` value. |
| `130` | Second Ctrl+C within 5 seconds (`128 + SIGINT`). Hard exit; no drain. |

## Signal handling

`aviso listen` installs a Ctrl+C handler. The first signal triggers graceful drain: the supervisor's existing drop cascade closes the watch, in-flight commits land, then the CLI exits 0 (or 1 if any listener errored earlier). A second Ctrl+C within 5 seconds calls `std::process::exit(130)` directly, bypassing the drain.

## `--from <VALUE>` formats

Both `aviso replay --from <VALUE>` (mandatory) and `aviso listen --from <VALUE>` (optional override; when set, applies uniformly to every resolved listener and overrides any per-YAML `from_id` / `from_date`) accept seven input forms, tried in this order:

1. **Pure-digit `u64`** -> sequence id. Examples: `42`, `1234567`.
2. `YYYY-MM-DD` -> midnight UTC. Example: `2024-01-15`.
3. `"YYYY-MM-DD HH:MM"` (space separator, **quotes required**) -> UTC. Example: `"2024-01-15 14:30"`.
4. `"YYYY-MM-DD HH:MM:SS"` (space separator, **quotes required**) -> UTC.
5. `YYYY-MM-DDTHH:MM:SS` (T separator) -> UTC.
6. `YYYY-MM-DDTHH:MM:SSZ` (T separator, explicit Z) -> UTC.
7. `YYYY-MM-DDTHH:MM:SS.ffffffZ` (pyaviso strict, six fractional digits + Z) -> UTC.

All six date forms normalise to the wire format `YYYY-MM-DDTHH:MM:SS.ffffffZ` (exactly six fractional digits, microsecond precision) before being sent to the server. Inputs with fewer fractional digits are zero-padded; nine-digit input is truncated to six.

**Important ambiguity rule**: pure digits are ALWAYS routed as sequence id. The compact `YYYYMMDD` form (`20240115`) is therefore a sequence id, NOT a date. To pass a date, use the dashed form: `--from 2024-01-15`.

### Precedence vs. the persisted state cursor

This subsection applies to `aviso listen` only. `aviso replay` is stateless (it never reads or writes the state file) and always honors the operator-supplied `--from <VALUE>` verbatim; the listener YAML's `from_id` / `from_date` defaults are ignored when `--from` is set on the command line.

When `--from <VALUE>` is supplied AND a state-store cursor exists for the same listener:

- **`--from` always wins for the initial seek.** The supervisor uses your value and ignores the stored cursor.
- **The state file is protected from regression.** As the rewind redelivers notifications, the monotonic-merge rule discards any `put` whose sequence is less than or equal to what is already on disk. The file's high-water mark is preserved; you cannot accidentally rewind the cursor by passing an early `--from`.
- **Once the run advances past the previous high-water mark, the file resumes normal updates.**
- **Restarting without `--from` honours the stored cursor again.**

The implied operator workflow is: pass `--from` as a *one-shot* manual rewind, then remove it from subsequent invocations. Leaving `--from <date>` in a systemd unit will redeliver from that date on every restart, because the supervisor re-evaluates precedence at every process start.

See [The state file: `--from` interaction](../resume/state-file.md#-from-interaction) for the full breakdown with worked sequences.

## TLS configuration

The `aviso` CLI talks to `aviso-server` over HTTPS in production. Two flags govern TLS validation:

### `--ca-bundle <PATH>`: trust an internal CA

Use this when `aviso-server` is fronted by a TLS endpoint whose certificate is signed by an **internal CA** not in the system trust store. Common scenarios:

- A private deployment behind a corporate root certificate authority.
- A self-hosted cluster running its own ACME (e.g. step-ca, smallstep, an internal kubernetes ingress with cert-manager backed by a private issuer).
- A development environment running aviso-server behind a self-signed cert from a custom CA you control.

The flag is **repeatable**: pass `--ca-bundle PATH1 --ca-bundle PATH2` for multiple certificates. Each PEM file is read once at CLI startup, parsed via `reqwest::Certificate::from_pem`, and added to the HTTP client's root certificate store. The system trust store stays in effect; `--ca-bundle` only **adds**, never replaces.

**Step-by-step**:

1. Get the CA certificate in PEM form from your aviso-server operator. The file looks like:

   ```text
   -----BEGIN CERTIFICATE-----
   MIIDDTCCAfWgAwIBAgIUOoEsjJSbNYUFzrZ...
   ...
   -----END CERTIFICATE-----
   ```

2. Save it somewhere readable, e.g. `~/.config/aviso/internal-ca.pem`.

3. Pass it on the command line:

   ```bash
   aviso \
     --base-url https://aviso.internal.example \
     --token "$AVISO_TOKEN" \
     --ca-bundle ~/.config/aviso/internal-ca.pem \
     schema list
   ```

4. Or set it in the config file:

   ```yaml
   tls:
     ca_bundle: ["/home/alice/.config/aviso/internal-ca.pem"]
   ```

5. If your private deployment uses an intermediate CA, pass BOTH the intermediate and the root certificates, either as separate `--ca-bundle` flags or as a single PEM file containing both blocks.

**How to fetch a PEM from a running server**:

```bash
openssl s_client -connect aviso.internal.example:443 -showcerts < /dev/null \
  2>/dev/null | sed -n '/-----BEGIN CERTIFICATE-----/,/-----END CERTIFICATE-----/p' \
  > internal-ca.pem
```

Verify the file parses as a certificate before using it:

```bash
openssl x509 -in internal-ca.pem -noout -subject -issuer
```

### `--danger-accept-invalid-certs`: bypass validation (insecure)

**Insecure by design**. Intended ONLY for short-lived development against a self-signed `aviso-server` when shipping the cert via `--ca-bundle` is not practical. With this flag set:

- TLS certificate validation is disabled entirely. Any certificate (or no certificate at all on the wire, modulo what the TLS handshake itself requires) is accepted.
- A `WARN` log fires at session startup with the stable event name `cli.tls.insecure_mode`. Log scrapers can use that to flag misuse in production audit trails.

```bash
aviso \
  --base-url https://localhost:8443 \
  --danger-accept-invalid-certs \
  schema list
```

**Recommended fallback ordering**:

1. **System trust store** (default; no flags needed). Works for any aviso-server fronted by a publicly-trusted CA (Let's Encrypt, commercial CAs in the OS bundle).
2. **`--ca-bundle <PATH>`**. Works for self-hosted aviso-server behind a private CA. The right production move whenever the server is on the network for more than a one-off test.
3. **`--danger-accept-invalid-certs`**. Only as a last resort, only in development. Never in production. Pair with `tracing` log monitoring so the WARN is visible.

The flag is also accepted in the config file:

```yaml
tls:
  danger_accept_invalid_certs: true
```

Setting it in a config file is a particularly bad idea (the insecure default propagates across invocations); the WARN still fires every time. The flag exists in YAML form so operators with structured-config tooling can express intent, not because we recommend it.

## Shell completions

```bash
# Bash: install into the user's completions directory
aviso completions bash > ~/.local/share/bash-completion/completions/aviso

# Zsh: install into fpath
aviso completions zsh > ~/.local/share/zsh/site-functions/_aviso

# Fish:
aviso completions fish > ~/.config/fish/completions/aviso.fish

# PowerShell: write into $PROFILE
aviso completions powershell >> $PROFILE

# Elvish:
aviso completions elvish > ~/.config/elvish/lib/aviso.elv
```

Restart the shell (or `source` the file) for completions to take effect.

## Troubleshooting

**`error: aviso admin wipe-all requires --yes`**

The destructive admin commands require explicit confirmation. Add `--yes` to acknowledge. The flag is per-leaf-command only and cannot be set in the config file by design (a config-file `--yes` would defeat the protection).

**`error: no listeners to run`**

Either pass listener YAML files as positional arguments to `aviso listen`, or add a `listeners:` block to the global config (`~/.config/aviso/config.yaml` by default).

**`--from 20240115` resolves as sequence id 20240115, not the date 15 January 2024**

Pure-digit input is always treated as a sequence id (Amendment H ambiguity rule). The CLI accepts the compact eight-digit input without error, but routes it through `from_id`, not `from_date`. To pass a date, use the dashed form `--from 2024-01-15`. Verify the routed cursor by running with `-v` (so the `aviso` crate's DEBUG events fire) and checking the tracing output for `cli.replay.notification` events: sequence numbers near your starting id indicate id routing; events from the date you meant indicate date routing.

**The CLI loops indefinitely on `aviso listen` against a finite SSE stream**

Expected. In watch mode the lib's resilience layer reconnects on `connection-closing` events. A finite mock SSE stream loops forever from the client's perspective. To stop the listener, send Ctrl+C (first signal graceful drain, second within 5s hard-exits 130).

**`event.name=cli.tls.insecure_mode` WARN fires on every CLI invocation**

The `--danger-accept-invalid-certs` flag (or the `tls.danger_accept_invalid_certs: true` config value) is set. Switch to `--ca-bundle` per the [TLS configuration](#tls-configuration) section to silence the WARN and restore certificate validation.

**Token / password values appearing in tracing logs**

They shouldn't. The lib's `Bearer::Debug` and `Basic::Debug` redact secrets, and `--token` / `--password` flag values are masked under `aviso config dump --redact`. If you see a secret in a tracing event, file a bug; that's a regression.

## See also

- [Watch streams overview](../watch/overview.md): the streaming architecture the `listen` and `replay` subcommands consume.
- [Authentication](./auth.md): how the lib's auth providers work; the CLI composes `Bearer` / `Basic` from the layered config.
- [Architecture](../internals/architecture.md): the layering of the CLI on top of the core library.
- [The state file](../resume/state-file.md): annotated on-disk format and `--from` interaction.
