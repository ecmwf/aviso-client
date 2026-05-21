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
  notify "event=mars,class=od,stream=oper,type=fc,data={\"region\":\"north\"}"
```

The single positional argument is a comma-separated `key=value` list (pyaviso parity). Only `event=<TYPE>` is mandatory. The optional `data=<JSON>` key becomes the notification payload; every other `key=value` pair enters the identifier map.

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
| `aviso listen [FILES...]` | Run one or more listeners concurrently | Empty stdout; triggers handle output |
| `aviso replay --from <VALUE> [FILES...]` | Replay historical notifications from a cursor (positional listener YAMLs, `--listener <NAME>` to pick one from the resolved set, or `--event <TYPE> --identifiers <JSON>` for ad-hoc) | Empty stdout; triggers handle output |
| `aviso schema list` | GET `/api/v1/schema` | TTY: table; pipe / `--json`: NDJSON |
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
state_file: "/path/to/state.json"  # optional; default ~/.config/aviso/state.json
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

The listeners list is pyaviso-compatible with one rename: pyaviso's `request:` is `identifiers:` here (matching the lib's `Notification::identifier` field). Trigger configs reuse the library's `TriggerConfig` deserialiser so all four shipped trigger kinds (`echo`, `log`, `command` on Unix, `webhook`) work from YAML unchanged.

### Positional listener files

`aviso listen file1.yaml file2.yaml ...` accepts a variadic positional list of listener YAML files. Each carries its own top-level `listeners:` list. Positional files **REPLACE** (not merge with) the global config's `listeners:` for the invocation. With no positional files and no global `listeners:`, the CLI exits with code `2` and a stderr message naming both paths checked.

## Environment variables

| Variable | Effect |
|---|---|
| `AVISO_LOG` | `tracing_subscriber` `EnvFilter` directive. Default `INFO`. `-v` / `-vv` override. |
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

Pure-digit input is always treated as a sequence id (Amendment H ambiguity rule). The CLI accepts the compact eight-digit input without error, but routes it through `from_id`, not `from_date`. To pass a date, use the dashed form `--from 2024-01-15`. Verify the routed cursor by checking the tracing output at INFO level for `cli.replay.notification` events (sequence numbers near your starting id indicate id routing; events from the date you meant indicate date routing).

**The CLI loops indefinitely on `aviso listen` against a finite SSE stream**

Expected. In watch mode the lib's resilience layer reconnects on `connection-closing` events. A finite mock SSE stream loops forever from the client's perspective. To stop the listener, send Ctrl+C (first signal graceful drain, second within 5s hard-exits 130).

**`event.name=cli.tls.insecure_mode` WARN fires on every CLI invocation**

The `--danger-accept-invalid-certs` flag (or the `tls.danger_accept_invalid_certs: true` config value) is set. Switch to `--ca-bundle` per the [TLS configuration](#tls-configuration) section to silence the WARN and restore certificate validation.

**Token / password values appearing in tracing logs**

They shouldn't. The lib's `Bearer::Debug` and `Basic::Debug` redact secrets, and `--token` / `--password` flag values are masked under `aviso config dump --redact`. If you see a secret in a tracing event, file a bug; that's a regression.

## See also

- [Watch streams overview](../watch/overview.md): the streaming architecture the `listen` and `replay` subcommands consume.
- [Authentication](./auth.md): how the lib's auth providers work; the CLI composes `Bearer` / `Basic` from the layered config.
- [Architecture](../internals/architecture.md) and [Architectural decisions](../internals/decisions.md) (D8 auth refresh, D12 logging, D17 from-date contract, D19 watch API, D20 multi-listener semantics).
