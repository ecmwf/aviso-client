# Listener YAML reference

The file format `aviso listen` (and `aviso replay` with positional files) reads.

A listener file has one top-level key, `listeners:`, with a list of listener definitions. The same shape is accepted inside the main config file (`~/.config/aviso/config.yaml`).

## Minimal example

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
    triggers:
      - type: echo
```

## All fields

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | string | no | unnamed | Label for logs and the echo trigger's leader line. |
| `event` | string | yes | | Event type to subscribe to. Must match a type the server publishes. |
| `identifiers` | map | no | `{}` | Filter. The server returns only notifications whose identifier matches every field here. Empty map allowed; the server may still require certain fields per the schema. |
| `triggers` | list | no | `[]` | Triggers to run for each matching notification. Empty list is accepted, but a useful listener normally has at least one trigger. |
| `from_id` | integer | no | unset | Default starting sequence for the first connection. Overridden by `--from`. |
| `from_date` | string | no | unset | Default starting ISO-8601 datetime. The value is sent verbatim to the server, so it must be in a form the server accepts: `YYYY-MM-DDTHH:MM:SSZ`, `YYYY-MM-DDTHH:MM:SS.ffffffZ`, or `YYYY-MM-DD HH:MM:SS+HH:MM`. A bare `YYYY-MM-DD` is **not** accepted here (the CLI's `--from` flag does that normalisation, but the YAML field does not). Mutually exclusive with `from_id`. |

## Trigger fields

Every trigger entry has a `type:` field plus per-kind fields. Shared options:

| Field | Type | Default | Applies to | Description |
|---|---|---|---|---|
| `retries` | integer | `0` | all | Additional attempts after the first failure. Total attempts = `retries + 1`. |
| `required` | boolean | `true` | all | When `true`, a final failure stops the listener. When `false`, the failure logs a `WARN` and the listener keeps running. |
| `timeout` | duration | `30s` for HTTP triggers; absent for command | command, webhook, teams, post | Per-attempt wall clock. [humantime](https://docs.rs/humantime/) syntax: `30s`, `2m`, `1h30m`, `500ms`. |
| `fail_fast` | boolean | `true` | command, webhook, teams, post | When `true`, deterministic failures (4xx, template errors, non-zero command exit) bypass the retry budget. When `false`, every failure is retryable. |

### `echo`

```yaml
- type: echo
```

No required fields. Writes the notification to stdout (pretty JSON on a TTY, NDJSON otherwise).

### `log`

```yaml
- type: log
  path: /var/log/aviso/mars.log
```

| Field | Required | Description |
|---|---|---|
| `path` | yes | Absolute path to a file. The parent directory must exist. |

Appends one line of compact JSON per notification.

### `command`

```yaml
- type: command
  command: "./on-event.sh {{ notification.event_type }} {{ notification.sequence }}"
  env:
    DATA_DIR: /var/lib/aviso/data
  working_dir: /var/lib/aviso
  timeout: 30s
```

| Field | Required | Description |
|---|---|---|
| `command` | yes | Shell command to run. Goes through the template engine. |
| `env` | no | Extra environment variables, applied on top of the dispatcher-injected `AVISO_*` set. Values are not template-rendered. |
| `working_dir` | no | Directory to spawn the shell in. |

Unix only.

### `webhook`

```yaml
- type: webhook
  url: "https://hooks.example/notify"
  method: POST
  headers:
    Authorization: "Bearer {{ env.WEBHOOK_TOKEN }}"
  body_template: '{"event": "{{ notification.event_type }}", "seq": {{ notification.sequence }}}'
```

| Field | Required | Default | Description |
|---|---|---|---|
| `url` | yes | | Template-rendered URL. |
| `method` | no | `POST` | One of `GET`, `POST`, `PUT`, `PATCH`, `DELETE`. Uppercase. |
| `headers` | no | | Map of header name → template-rendered value. |
| `body_template` | no | the compact JSON of the notification | Template-rendered body. |

### `teams`

```yaml
- type: teams
  url: "{{ env.TEAMS_WEBHOOK_URL }}"
  title_template: "Custom {{ notification.event_type }}"
```

| Field | Required | Default | Description |
|---|---|---|---|
| `url` | yes | | Teams Workflows webhook URL. |
| `title_template` | no | `aviso {{ notification.event_type }} #{{ notification.sequence }}` | Template-rendered title. |

Builds the Adaptive Card body automatically from the notification.

### `post`

```yaml
- type: post
  url: "https://receiver.example/aviso"
  headers:
    Authorization: "Bearer {{ env.RECEIVER_TOKEN }}"
```

| Field | Required | Default | Description |
|---|---|---|---|
| `url` | yes | | Receiver URL. |
| `headers` | no | | Map of header name → template-rendered value. |

Body is always the server's original CloudEvent envelope. No `body_template`, no `method` (always `POST`).

## Templates

The template engine for trigger fields is documented at [Triggers: template engine](../triggers/template-engine.md). Two namespaces inside `{{ ... }}`:

- `{{ notification.<dotted.path> }}`: a field of the notification.
- `{{ env.<NAME> }}`: a process environment variable.

## Resolution and precedence

When you run `aviso listen`:

1. Positional YAML files replace the global config's `listeners:` for this invocation. They do not merge with it.
2. With multiple positional files, the listener lists are concatenated in argv order.
3. With no positional files, the global config's `listeners:` block is used.
4. With no listeners anywhere, `aviso listen` exits with code 2.

`--event` and `--identifiers` (the inline mode) take precedence over positional YAML files when both are present.

## What next

- [Trigger overview](../triggers/overview.md): pick the right trigger.
- [CLI publish and listen](../cli/publish-and-listen.md): running listeners.
