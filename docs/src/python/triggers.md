# Triggers

Triggers are declarative side effects that run per notification: write to a
file, post to a webhook, post to a Microsoft Teams channel, run a shell command.
The Python `Trigger` class wraps the same six kinds the Rust core ships, with
the same semantics: retries, optional-vs-required, fail-fast.

Triggers attach to a watch via the `triggers=` kwarg on `client.listen(...)`.
The supervisor dispatches each trigger for each notification before the iterator
yields it. A required trigger that fails after retries stops the watch with
`aviso.TriggerError`.

The listener example below uses `test_polygon` as the event type. If your server
does not have it configured, replace the event type and identifier fields with
one of your own. See
[What is on your server](./quickstart.md#what-is-on-your-server) in the
quickstart for how to discover what is configured.

## A complete listener with triggers

The watch below prints every notification (`echo`) and also appends it to a log
file (`log`). Stop with Ctrl+C.

```python
"""Listen with two triggers: print to stdout and append to a log file."""

import os
import pathlib
import tempfile
import aviso

log_path = pathlib.Path(tempfile.gettempdir()) / "aviso-doc-example.log"

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

print(f"writing log to {log_path}")
with client.listen(
    "test_polygon",
    filter={"polygon": "0,0,1,0,1,1,0,0"},
    triggers=[aviso.Trigger.echo(), aviso.Trigger.log(log_path)],
) as iterator:
    for _ in iterator:
        pass  # the echo trigger already printed; the log trigger already wrote
```

Each notification produces one line on stdout (from echo) and one line in
`log_path` (from log). Both are compact JSON, one object per line. If you want
to build the watch once and reuse it across several `listen()` calls, see
[Reusing a watch request](./listen.md#reusing-a-watch-request) for the builder
form.

## The six kinds

### Echo

Writes one line of compact JSON per notification to standard output. The
simplest way to see what is arriving in a stream.

```python
import aviso

aviso.Trigger.echo()
aviso.Trigger.echo(label="test-stream")  # adds a label leader line on TTY
```

### Log

Appends one line of compact JSON per notification to a file. The file opens
lazily on first dispatch.

```python
import aviso

aviso.Trigger.log("/var/log/aviso/test-polygon.log")
aviso.Trigger.log("/var/log/aviso/test-polygon.log", retries=2, required=False)
```

### Command (Unix only)

Runs `/bin/sh -c <rendered_command>` per notification. The notification's fields
are injected as `AVISO_*` environment variables; the command string can also
reference them via `{{ notification.<dotted.path> }}` and `{{ env.<NAME> }}`
templates.

<!-- not-runnable -->
```python
import aviso

aviso.Trigger.command(
    "./process.sh {{ notification.identifier.date }}",
    env={"DEST": "/data"},
    working_dir="/srv/jobs",
    retries=2,
    timeout=300.0,
)
```

On non-Unix builds, the constructor raises `aviso.ConfigError`.

### Webhook

Sends an HTTP request per notification to a configured URL. URL, header values,
and body all run through the template engine.

<!-- not-runnable -->
```python
import aviso

aviso.Trigger.webhook(
    "https://hooks.example.org/notify",
    method=aviso.HttpMethod.POST,
    headers={"Authorization": "Bearer {{ env.HOOK_TOKEN }}"},
    body_template='{"sequence": "{{ notification.sequence }}"}',
)
```

The default method is `POST` with a compact JSON body of the full notification.
The default timeout is 30 seconds.

### Teams

A webhook with an auto-built Adaptive Card body, aimed at Microsoft Teams
workflow webhooks.

<!-- not-runnable -->
```python
import aviso

aviso.Trigger.teams("https://prod-x.westeurope.logic.azure.com/workflows/...")
```

### Post

A webhook that forwards the raw server-emitted CloudEvent envelope rather than
the library's narrowed `Notification` view. Use it when a downstream consumer
expects the full CloudEvent shape.

<!-- not-runnable -->
```python
import aviso

aviso.Trigger.post("https://collector.example.org/events")
```

## Tunables

Every trigger accepts the same four tunables, either as keyword arguments to the
constructor or as chainable setters:

```python
import aviso

aviso.Trigger.echo(retries=3, required=False)
aviso.Trigger.echo().retries(3).required(False)
```

- `retries`: number of additional attempts after the first failure. Default `0`.
- `required`: a required trigger terminates the watch on failure; an optional
  trigger logs a `WARN` and the watch continues. Default `True`.
- `timeout`: meaningful for command and HTTP-based triggers; silently ignored on
  echo and log.
- `fail_fast`: meaningful for command and HTTP-based triggers; treats
  deterministic failures (non-zero exit, 4xx HTTP) as terminal. Default `True`.

## When to use which

| You want to | Use |
|---|---|
| Eyeball what is arriving | `echo` |
| Keep an audit log of every delivery | `log` |
| Kick off a shell pipeline | `command` |
| Push to a generic HTTP receiver | `webhook` |
| Push to a Microsoft Teams channel | `teams` |
| Forward raw CloudEvents to a collector | `post` |
| Run arbitrary Python code per notification | (use the iteration loop body, no trigger needed) |

In-process Python logic belongs in the iteration loop body. Triggers exist for
declarative durable side effects that should keep working when the calling
program exits.

## With `AsyncAvisoClient`

The Trigger class is a value type; it has no async surface of its own. Pass the
same `triggers=[...]` list to either client.
