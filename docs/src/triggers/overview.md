# Triggers

A trigger is a per-notification side-effect attached to a listener. When a
notification matches the listener's filter, every configured trigger runs in
declaration order. The core has six built-in trigger kinds:

| Kind | What it does | Use when |
|---|---|---|
| [echo](./echo.md) | Writes the notification to stdout (pretty JSON on TTY, NDJSON in pipe) | Interactive inspection; piping into `jq` / file ingestion |
| [log](./log.md) | Appends NDJSON to a file | Local persistence; audit trails; later batch processing |
| [command](./command.md) | Runs `/bin/sh -c <rendered>` with `AVISO_*` env vars set (Unix only) | Custom scripts; CLI integration; anything you can put in a shell |
| [webhook](./webhook.md) | Generic HTTP request with custom URL / headers / method / body | Any REST endpoint; full control over the request shape |
| [teams](./teams.md) | HTTP POST that auto-builds a Microsoft Teams Adaptive Card | Teams channels via Workflows / Power Automate |
| [post](./post.md) | HTTP POST that forwards the server's CloudEvent verbatim | pyaviso migration; generic CloudEvent receivers |

All six triggers share the same dispatch contract: retry budget,
required-vs-optional, timeout, and fail-fast policy. The
[template engine](./template-engine.md) is shared by `command`, `webhook`,
`teams`, and `post`.

## Configuring triggers in listener YAML

A listener block carries a `triggers:` array. Each entry must have a `type:`
field; the other fields depend on the trigger kind.

```yaml
listeners:
  - name: my-listener
    event: mars
    identifiers:
      class: od
    triggers:
      - type: echo
      - type: webhook
        url: "https://hooks.example.com/notify"
        headers:
          Authorization: "Bearer {{ env.HOOK_TOKEN }}"
```

When the listener receives a matching notification, both triggers run
sequentially: echo prints to stdout, then webhook POSTs the notification.

## Shared trigger options

`retries` and `required` are accepted by every trigger kind. `timeout` and
`fail_fast` are accepted only by `command`, `webhook`, `teams`, and `post`; the
YAML loader rejects them on `echo` and `log` (those triggers do not have a
meaningful timeout or fail-fast concept).

| Field | Type | Default | Accepted by |
|---|---|---|---|
| `retries` | integer | `0` | All kinds |
| `required` | boolean | `true` | All kinds |
| `timeout` | duration | `30s` for webhook/teams/post; absent for command | command, webhook, teams, post |
| `fail_fast` | boolean | `true` | command, webhook, teams, post |

Field meanings:

- `retries`: additional attempts after the first failure. Total attempts =
  `retries + 1`. Backoff between attempts uses the supervisor's standard
  exponential schedule with full jitter.
- `required`: when `true`, a final failure terminates the listener with
  `ClientError::TriggerFailed`. When `false`, the failure is logged at `WARN`
  and the listener continues.
- `timeout`: per-trigger wall clock. Parsed as a humantime string (`30s`, `2m`,
  `1h30m`, `500ms`).
- `fail_fast`: when `true`, deterministic failures bypass the retry budget. When
  `false`, every failure is retryable up to the `retries` budget.

A deterministic failure is one that produces the same outcome on every retry
with the same notification and environment:

- For `command`: non-zero exit code, template render error.
- For `webhook`, `teams`, and `post`: 4xx HTTP status, template render error,
  invalid request setup.

Transient failures (5xx responses, transport errors, timeouts, I/O errors) use
the retry budget regardless of `fail_fast` because they can succeed on a retry.

## Order, atomicity, and failure semantics

Triggers run **sequentially** in declaration order. aviso does not run triggers
for the same notification in parallel.

During retry waits and between triggers, aviso honours Ctrl+C and exits cleanly.
It does not interrupt a trigger halfway through its current attempt.

A `required: true` trigger that fails terminates that listener after exhausting
retries. Other listeners in the same `aviso listen` invocation continue running.

A `required: false` trigger that fails logs a warning and the listener
continues.

## At-least-once delivery and trigger ordering

The supervisor advances the resume cursor (`last_committed_sequence` in
[the state file](../reference/state-file.md)) **only after all required triggers
for a notification succeed**. This means:

- If a notification has 3 triggers and only the first succeeds when the listener
  crashes, the cursor does NOT advance. On restart, the listener redelivers the
  notification, and ALL THREE triggers run again. Operators must design triggers
  to be idempotent.
- Optional triggers (`required: false`) do not block cursor advancement. A
  failed optional trigger does not cause redelivery.
- Triggers run BEFORE the notification is checkpointed to the state file. The
  trigger's side-effects are durable-before-cursor-advance.

## Picking the right trigger

| If you want to | Use |
|---|---|
| See notifications in your terminal during interactive testing | `echo` |
| Tail notifications into a local file | `log` |
| Run an arbitrary shell command per notification | `command` |
| POST to any HTTP endpoint with full control | `webhook` |
| Send to a Microsoft Teams channel | `teams` |
| Send an email per notification | `command` or `webhook`, see [Sending email](./email.md) |
| Forward the unmodified CloudEvent that aviso-server emitted | `post` |
| Forward to multiple of the above | Combine - listeners accept multiple triggers |
