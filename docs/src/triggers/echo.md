# Echo trigger

Writes the notification to standard output. Format adapts to whether stdout is a TTY or a pipe; this is the canonical dual-mode shape every other interactive aviso surface follows.

## YAML

```yaml
triggers:
  - type: echo
    retries: 0          # optional, default 0
    required: true      # optional, default true
```

No other configurable fields. The YAML loader rejects `timeout` and `fail_fast` at config-load time (`EchoConfig` is `#[serde(deny_unknown_fields)]`) because the echo trigger has no meaningful timeout (a single-call buffered write completes immediately) and no meaningful fail-fast distinction (the failure modes are deterministic-on-environment, so retrying never helps).

## TTY output (interactive)

```text
new notification (listener: my-listener, trigger: echo):
{
  "event_type": "mars",
  "sequence": 42,
  "identifier": {
    "class": "od",
    "date": "20260601",
    ...
  },
  "payload": { ... }
}
```

The leader line is rendered in dark gray (ANSI `\x1b[90m`, bright-black) when color is enabled. The JSON body is plain text so it stays copy-paste-friendly into `jq` and similar tools even with `--color always`.

The `(listener: <name>, ...)` segment appears when the listener has an explicit `name:` in its YAML. Multi-listener configurations watching the same event_type can interleave deliveries on shared stdout; the leader names the listener so operators can attribute each line. A single-listener config or a listener without `name:` shows the bare leader `new notification (trigger: echo):`.

## Pipe output (machine consumers)

When stdout is **not** a TTY (piped, redirected to file, captured by a downstream tool), the echo trigger emits one line of compact NDJSON per notification:

```text
{"event_type":"mars","sequence":42,"identifier":{"class":"od",...},"payload":{...}}
```

No leader line, no ANSI escapes, no whitespace, exactly one notification per line. This is the contract `aviso listen | jq` and `aviso listen >> notifications.ndjson` rely on.

The pipe-mode JSON shape is **identical** to what `serde_json::to_string(&notification)` would produce in the lib. Downstream consumers that copy-paste the TTY body into `jq` get the same shape with whitespace.

## Color control

The CLI's global `--color` flag selects from three modes (interacts with the [`NO_COLOR`](https://no-color.org/) environment variable per its convention):

| Mode | Behavior |
|---|---|
| `--color auto` (default) | Color enabled on TTY, disabled in pipe; respects `NO_COLOR=1` |
| `--color always` | Color enabled always (still no color in pipe-mode body since pipe mode emits compact NDJSON, not the human format) |
| `--color never` | No color anywhere |

Note: `--color always` does **not** force the human format on pipe-mode output. Pipe mode unconditionally emits compact NDJSON because the JSON body would be corrupted by ANSI escapes for any downstream `jq`-style consumer. If you want to force human format despite stdout being a pipe, redirect `stderr` instead (and have the trigger write there) - but no such feature exists in aviso today; pipe mode is for machine consumers.

## Failure modes

Echo writes a buffer-prepared single-line NDJSON record in one I/O call against locked stdout. The only failure is `io::Error` (`Broken pipe` when the downstream consumer closes its read end, `ENOSPC` on a full disk if stdout is redirected to a file). Errors surface as `TriggerError::Io`.

Echo is always considered terminal-failure-free in the retry sense: `fail_fast` doesn't apply, and the failure modes that can occur are I/O errors which are nominally retryable but pragmatically not (a broken pipe stays broken).

## When to use

- Interactive testing: `aviso listen` in a terminal, eyeballing notifications.
- Pipelines: `aviso listen | jq '.payload'` to extract a field across notifications.
- Bulk capture for analysis: `aviso listen > notifications.ndjson`.

## When NOT to use

- Long-running production logging: use [`log`](./log.md) (file-backed, append-only).
- Forwarding to a remote service: use [`webhook`](./webhook.md), [`teams`](./teams.md), or [`post`](./post.md).
