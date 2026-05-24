# Echo trigger

Writes the notification to standard output. Format adapts to whether stdout is a terminal or a pipe, so it works for both humans and tools.

## Quick start without a YAML file

`aviso listen --event <TYPE> --identifiers <JSON>` runs a single ad-hoc listener with a default echo trigger, no YAML required:

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
aviso listen --event mars --identifiers '{"class":"od"}' | jq -r '.payload'
```

See [Publish and listen: listen with inline flags](../cli/publish-and-listen.md#listen-with-inline-flags) for flag pairing, precedence, state store behaviour, and `--from`.

## YAML

```yaml
triggers:
  - type: echo
    retries: 0          # optional, default 0
    required: true      # optional, default true
```

No other configurable fields. `timeout` and `fail_fast` are not accepted because echo has no network call or child process to time out.

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

The leader line uses muted color when color is enabled. The JSON body stays plain text so it can be copied into `jq` and similar tools.

The `(listener: <name>, ...)` segment appears when the listener carries a name. Multi-listener configurations watching the same event_type can interleave deliveries on shared stdout; the leader names the listener so operators can attribute each line. The label comes from three places, in priority order:

- An explicit `name:` field on the listener in its YAML (`name: mars-od` → `(listener: mars-od, trigger: echo)`).
- The fixed string `ad-hoc` when the listener was built via inline mode (`aviso listen --event ... --identifiers ...`). The startup banner reads `Listening for ad-hoc [mars] (class=od). Press Ctrl+C to stop.` and every echo leader carries `(listener: ad-hoc, trigger: echo)`.
- Nothing (the bare leader `new notification (trigger: echo):`) when the listener has no `name:` in its YAML.

## Pipe output (machine consumers)

When stdout is **not** a TTY (piped, redirected to file, captured by a downstream tool), the echo trigger emits one line of compact NDJSON per notification:

```text
{"event_type":"mars","sequence":42,"identifier":{"class":"od",...},"payload":{...}}
```

No leader line, no ANSI escapes, no whitespace, exactly one notification per line. This is the contract `aviso listen | jq` and `aviso listen >> notifications.ndjson` rely on.

Pipe-mode JSON has the same fields as the TTY body, only without whitespace.

## Color control

The CLI's global `--color` flag selects from three modes (interacts with the [`NO_COLOR`](https://no-color.org/) environment variable per its convention):

| Mode | Behavior |
|---|---|
| `--color never` (default) | No color anywhere; no ANSI escapes emitted |
| `--color auto` | Color enabled on TTY, disabled in pipe; respects `NO_COLOR=1` |
| `--color always` | Color enabled always (still no color in pipe-mode body since pipe mode emits compact NDJSON, not the human format) |

Note: `--color always` does **not** force the human format when stdout is piped. Pipe mode always emits compact NDJSON because downstream tools expect clean JSON.

## Failure modes

Echo can fail only when stdout cannot be written, for example when a downstream pipe closes or a redirected file is on a full disk.

Retry settings rarely help echo failures. A closed pipe or full disk usually needs an operator fix before the same write can succeed.

## When to use

- Interactive testing: `aviso listen` in a terminal, eyeballing notifications.
- Pipelines: `aviso listen | jq '.payload'` to extract a field across notifications.
- Bulk capture for analysis: `aviso listen > notifications.ndjson`.

## When NOT to use

- Long-running production logging: use [`log`](./log.md) (file-backed, append-only).
- Forwarding to a remote service: use [`webhook`](./webhook.md), [`teams`](./teams.md), or [`post`](./post.md).
