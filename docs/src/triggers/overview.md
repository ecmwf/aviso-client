<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Triggers

<div class="trigger-guide">

A trigger is a per-notification side-effect attached to a listener. When a
notification matches the listener's filter, every configured trigger runs in
declaration order. The core has six built-in trigger kinds:

| Kind | What it does | Use when |
|---|---|---|
| [echo](./echo.md) | Prints JSON | Inspect or pipe |
| [log](./log.md) | Appends NDJSON | Save to a file |
| [command](./command.md) | Runs a shell | Custom scripts |
| [webhook](./webhook.md) | Sends HTTP | REST endpoints |
| [teams](./teams.md) | Posts a card | Teams channels |
| [post](./post.md) | Posts a CloudEvent | Forward events |

- **Echo** writes pretty JSON on a terminal and NDJSON in a pipe, for tools
  such as `jq` or file ingestion.
- **Log** keeps a local file for audit trails or later batch processing.
- **Command** runs `/bin/sh -c <rendered>` with `AVISO_*` environment variables
  set. Use it for shell scripts or CLI integration. Unix only.
- **Webhook** lets you choose the URL, headers, method, and body of the HTTP
  request.
- **Teams** builds a Microsoft Teams Adaptive Card and sends it by HTTP POST
  through Workflows or Power Automate.
- **Post** forwards the server's CloudEvent by HTTP POST. Use it for pyaviso
  migration or generic CloudEvent receivers. See
  [Body shape](./post.md#body-shape) for how values and JSON formatting are
  preserved.

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

| Field | Type | Default |
|---|---|---|
| `retries` | integer | `0` |
| `required` | boolean | `true` |
| `timeout` | duration | See below |
| `fail_fast` | boolean | `true` |

Field meanings:

- `retries`: additional attempts after the first failure. Total attempts =
  `retries + 1`. Backoff between attempts uses the supervisor's standard
  exponential schedule with full jitter.
- `required`: when `true`, a final failure terminates the listener with
  `ClientError::TriggerFailed`. When `false`, the failure is logged at `WARN`
  and the listener continues.
- `timeout`: per-trigger wall clock. Parsed as a humantime string (`30s`, `2m`,
  `1h30m`, `500ms`). Defaults to `30s` for webhook, teams, and post. Absent by
  default for command.
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

</div>
