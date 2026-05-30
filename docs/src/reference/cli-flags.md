# CLI flags reference

Every flag for every subcommand. This is the long-form reference; for narrative
usage, start at [CLI overview](../cli/overview.md). Refresh this page whenever
the Clap command surface changes.

To see the same information from the binary itself:

```bash
aviso --help
aviso <SUBCOMMAND> --help
```

## Global flags

Available on every subcommand.

| Flag | Description |
|---|---|
| `-c, --config <PATH>` | Path to the YAML config file. Default `~/.config/aviso/config.yaml`. Env override: `AVISO_CLIENT_CONFIG_FILE`. |
| `--state-file <PATH>` | Path to the state file. Default `~/.config/aviso/state.json`. Env override: `AVISO_STATE_FILE`. |
| `--base-url <URL>` | Override the server URL. Env override: `AVISO_BASE_URL`. |
| `--token <TOKEN>` | Bearer auth token. Mutually exclusive with `--username`/`--password`. Env override: `AVISO_TOKEN`. |
| `--username <USERNAME>` | Basic auth username. Requires `--password`. Mutually exclusive with `--token`. Env override: `AVISO_USERNAME`. |
| `--password <PASSWORD>` | Basic auth password. Requires `--username`. Mutually exclusive with `--token`. Env override: `AVISO_PASSWORD`. |
| `--ca-bundle <PATH>` | PEM-encoded CA certificate to trust in addition to the system roots. Repeatable. See [Configuration: trust an internal CA](../cli/configuration.md#trust-an-internal-ca). |
| `--danger-accept-invalid-certs` | Disable TLS validation. Insecure; logs a `WARN` at startup. |
| `--json` | Force JSON output. Overrides the TTY-aware default. |
| `--color <auto\|always\|never>` | Color output mode. Default `never`. |
| `-v, --verbose` | Increase verbosity. Repeatable: `-v` for DEBUG, `-vv` for TRACE. Overridden by `AVISO_LOG` when set. |
| `-h, --help` | Print help. |
| `-V, --version` | Print version. |

## `aviso notify <PARAMETERS>`

Publish one notification to `/api/v1/notification`.

| Argument | Description |
|---|---|
| `<PARAMETERS>` | Comma-separated `key=value` list. `event=<TYPE>` is required; `data=<JSON>` is the optional payload; every other pair enters the identifier map. Wrap values containing commas in double quotes. |

Returns exit code 0 on success, 1 on a server error or network failure, 2 on
missing parameters.

## `aviso listen [LISTENER_FILES]...`

Run one or more listeners against `/api/v1/watch`.

| Argument / Flag | Description |
|---|---|
| `[LISTENER_FILES]...` | Listener YAML files. Each file's `listeners:` list is concatenated in argv order. Positional files replace (do not merge with) the global config's `listeners:` block for this invocation. |
| `--no-state-store` | Use an in-memory store for this invocation. Ignores any configured `state_file`. |
| `--from <VALUE>` | Cursor override applied uniformly to every resolved listener. Overrides per-YAML `from_id`/`from_date`. See [Configuration: `--from` value formats](../cli/configuration.md#from-value-formats). |
| `--event <TYPE>` | Inline ad-hoc listener: event type to listen for, without a YAML file. Requires `--identifiers`. Takes precedence over positional YAML files. |
| `--identifiers <JSON>` | Inline ad-hoc listener: identifiers filter as a JSON object. Requires `--event`. The inline listener runs with a single `echo` trigger. |

Returns 0 on a clean Ctrl+C, 1 if any listener task errored, 2 on no listeners
resolved.

## `aviso replay --from <VALUE> [LISTENER_FILES]...`

Replay historical notifications from a cursor.

| Argument / Flag | Description |
|---|---|
| `[LISTENER_FILES]...` | Listener YAML files. Same resolution as `aviso listen`. |
| `--listener <NAME>` | Pick one listener by name from the resolved set. Required when more than one listener resolves. |
| `--event <TYPE>` | Inline ad-hoc replay: event type, without a YAML file. Requires `--identifiers`. |
| `--identifiers <JSON>` | Inline ad-hoc replay: identifiers filter as a JSON object. Requires `--event`. |
| `--from <VALUE>` | **Required.** Sequence id or date to start replay from. |

Replay never touches the state file. Returns 0 on completion, 1 on error.

## `aviso schema list`

List event types the server knows about.

No subcommand-specific flags. On a TTY the output is a header line and a bullet
list of event types; piped or with `--json`, the output is NDJSON (one JSON
object per line).

## `aviso schema get <EVENT_TYPE>`

Fetch one schema as pretty JSON.

| Argument | Description |
|---|---|
| `<EVENT_TYPE>` | Event type name. |

## `aviso admin wipe-stream <EVENT_TYPE> --yes`

Delete every notification of one event type.

| Argument / Flag | Description |
|---|---|
| `<EVENT_TYPE>` | Event type whose notifications to delete. |
| `--yes` | **Required.** Confirms the destructive operation. |

## `aviso admin wipe-all --yes`

Delete every notification across every stream.

| Flag | Description |
|---|---|
| `--yes` | **Required.** Confirms the destructive operation. |

## `aviso admin delete <NOTIFICATION_ID> --yes`

Delete one notification.

| Argument / Flag | Description |
|---|---|
| `<NOTIFICATION_ID>` | The notification id in the `<event_type>@<sequence>` form. |
| `--yes` | **Required.** Confirms the destructive operation. |

## `aviso config dump`

Print the resolved configuration to stdout.

| Flag | Description |
|---|---|
| `--redact` | Mask tokens and passwords in the output. |
| `--json` | Force JSON output. The source-attribution comments become `source:` fields. |

## `aviso completions <SHELL>`

Print a shell completion script to stdout.

| Argument | Description |
|---|---|
| `<SHELL>` | One of `bash`, `zsh`, `fish`, `elvish`. |

## Environment variables

| Variable | Effect |
|---|---|
| `AVISO_LOG` | A [`tracing_subscriber`](https://docs.rs/tracing-subscriber) `EnvFilter` directive. When set, overrides `-v`/`-vv`. Useful recipes: `AVISO_LOG=warn,aviso=debug`, `AVISO_LOG=h2=debug,hyper=debug,aviso=debug`. |
| `NO_COLOR` | When set, suppresses ANSI colors in the `--color auto` mode. Per [no-color.org](https://no-color.org/). |
| `AVISO_CLIENT_CONFIG_FILE` | Config file path. Lower priority than `--config`. |
| `AVISO_STATE_FILE` | State file path. Lower priority than `--state-file`. |
| `AVISO_BASE_URL` | Server URL. Lower priority than `--base-url`. |
| `AVISO_TOKEN` | Bearer token. Lower priority than `--token`. |
| `AVISO_USERNAME` / `AVISO_PASSWORD` | Basic auth credentials. Lower priority than the flags. |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. A clean Ctrl+C with no prior listener failure also returns 0. |
| `1` | Runtime error: server returned 4xx/5xx, network failure, file I/O failure, or one or more `aviso listen` listener tasks errored or panicked. |
| `2` | Usage error: missing required flag, invalid argument value, destructive admin command without `--yes`, no listeners resolved for `aviso listen`, unparseable `--from` value. |
| `130` | Second Ctrl+C within 5 seconds (`128 + SIGINT`). Hard exit; no drain. |

## Signal handling

The first Ctrl+C triggers a graceful drain: aviso closes the watch connection,
flushes in-flight commits, then exits 0 (or 1 if a listener errored earlier). A
second Ctrl+C within five seconds calls the OS exit directly with code 130,
bypassing the drain.
