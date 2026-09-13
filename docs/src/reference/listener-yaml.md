# Listener YAML reference

The file format `aviso listen` (and `aviso replay` with positional files) reads.

A listener file has one top-level key, `listeners:`, with a list of listener
definitions. The same shape is accepted inside the main config file
(`~/.config/aviso/config.yaml`).

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
| `identifiers` | map | no | `{}` | Filter using supported fields from the event's schema. Conditions on those fields combine with AND. Empty map allowed only when the schema has no required filter fields. |
| `triggers` | list | no | `[]` | Triggers to run for each matching notification. Empty list is accepted, but a useful listener normally has at least one trigger. |
| `from_id` | integer | no | unset | Default starting sequence for the first connection. Overridden by `--from`. |
| `from_date` | string | no | unset | Default starting ISO-8601 datetime. The value is sent verbatim to the server, so it must be in a form the server accepts: `YYYY-MM-DDTHH:MM:SSZ`, `YYYY-MM-DDTHH:MM:SS.ffffffZ`, or `YYYY-MM-DD HH:MM:SS+HH:MM`. A bare `YYYY-MM-DD` is **not** accepted here (the CLI's `--from` flag does that normalisation, but the YAML field does not). Mutually exclusive with `from_id`. |

Identifier values may have any JSON-compatible YAML shape. Spatial values use
latitude first. This listener filters point-cloud notifications with a polygon:

```yaml
listeners:
  - name: alpine-observations
    event: observations
    identifiers:
      date: "20260601"
      polygon:
        - [46, 8]
        - [46, 9]
        - [47, 9]
        - [47, 8]
        - [46, 8]
    triggers:
      - type: echo
```

A point is `[latitude, longitude]`. Polygons need at least four pairs, with the
first pair repeated last. Providers send `point_cloud`; subscribers send
`polygon`. This uses the `observations` schema from
[Publish and listen](../cli/publish-and-listen.md).

## Constraint mappings {#constraints}

Use YAML mappings for constraints, not strings containing JSON. With the schema
and seed records from the
[weather tutorial](../cli/publish-and-listen.md#weather-constraints), put this
in `weather-listeners.yaml`:

```yaml
listeners:
  - name: selected-weather
    event: weather
    identifiers:
      date: "20260913"
      severity:
        gte: 5
      anomaly:
        between: [40, 50]
      region:
        in: [north, south]
    triggers:
      - type: echo
```

Quote the date so it stays a string. Leave numeric operands unquoted. This
selects B and C; the
[central operator reference](../concepts/filters.md#operators) explains the
allowed mappings.

Run the file directly, replaying the retained seeds before listening live:

```bash
aviso listen weather-listeners.yaml --from 0 --no-state-store
aviso replay weather-listeners.yaml --from 0
```

The listen command runs until Ctrl+C; run replay separately. Alternatively,
place the same `listeners:` block in your client config, `weather-config.yaml`,
alongside your connection settings. Select its listener by name for replay:

```bash
aviso --config weather-config.yaml listen --from 0 --no-state-store
aviso --config weather-config.yaml replay --listener selected-weather --from 0
```

Both config commands use the same mapping; they do not read the positional
listener file. `AVISO_BASE_URL` can supply the test server address for either
form.

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

No required fields. Writes the notification to stdout (pretty JSON on a TTY,
NDJSON otherwise).

### `log`

```yaml
- type: log
  path: /var/log/aviso/mars.log
```

| Field | Required | Description |
|---|---|---|
| `path` | yes | Path to a file. Resolves relative to the working directory of the `aviso` process when not absolute. The parent directory must exist; the trigger does not create directories. |

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

Body is always the server's original CloudEvent envelope. No `body_template`, no
`method` (always `POST`).

## Templates

The template engine for trigger fields is documented at
[Triggers: template engine](../triggers/template-engine.md). Two namespaces
inside `{{ ... }}`:

- `{{ notification.<dotted.path> }}`: a field of the notification.
- `{{ env.<NAME> }}`: a process environment variable.

## Resolution and precedence

When you run `aviso listen`:

1. Positional YAML files replace the global config's `listeners:` for this
   invocation. They do not merge with it.
2. With multiple positional files, the listener lists are concatenated in argv
   order.
3. With no positional files, the global config's `listeners:` block is used.
4. With no listeners anywhere, `aviso listen` exits with code 2.

`--event` and `--identifiers` (the inline mode) take precedence over positional
YAML files when both are present.

## What next

- [Trigger overview](../triggers/overview.md): pick the right trigger.
- [CLI publish and listen](../cli/publish-and-listen.md): running listeners.
