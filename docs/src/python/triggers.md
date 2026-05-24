# Triggers

Triggers run server-side-discovered side effects per notification: write to a file, post to a webhook, post to a Microsoft Teams channel, run a shell command, and so on. The Python `Trigger` class wraps the same six kinds the Rust core ships.

Attach triggers to a `WatchRequest`:

```python
import aviso

client = aviso.AvisoClient(base_url="https://aviso.example.org")
req = aviso.WatchRequest.watch("mars").with_triggers([
    aviso.Trigger.echo(),
    aviso.Trigger.log("/var/log/aviso/mars.log"),
])

for notification in client.listen(request=req):
    ...
```

When a required trigger fails after all its retries, the iteration raises `aviso.TriggerError` with structured attributes naming the trigger kind, the underlying error category, and the relevant payload fields (stderr tail for command, response status for webhook, and so on).

## The six kinds

### Echo

Writes one line of compact JSON per notification to standard output. The cheapest "see what's flowing" sink.

```python
aviso.Trigger.echo()
aviso.Trigger.echo(label="mars-od")           # prefixes a leader line on TTY
```

### Log

Appends one line of compact JSON per notification to a file. The file opens lazily on first dispatch.

```python
aviso.Trigger.log("/var/log/aviso/mars.log")
aviso.Trigger.log("/var/log/aviso/mars.log", retries=2, required=False)
```

### Command (Unix only)

Runs `/bin/sh -c <rendered_command>` per notification. The notification's fields are injected as `AVISO_*` environment variables; the command string can also reference them via `{{ notification.<dotted.path> }}` and `{{ env.<NAME> }}` templates.

```python
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

Sends an HTTP request per notification to a configured URL. URL, header values, and body all run through the template engine.

```python
aviso.Trigger.webhook(
    "https://hooks.example.org/notify",
    method=aviso.HttpMethod.POST,
    headers={"Authorization": "Bearer {{ env.HOOK_TOKEN }}"},
    body_template='{"sequence": "{{ notification.sequence }}"}',
)
```

The default method is `POST` with a compact JSON body of the full notification. The default timeout is 30 seconds.

### Teams

A webhook with an auto-built Adaptive Card body, aimed at Microsoft Teams workflow webhooks.

```python
aviso.Trigger.teams("https://prod-x.westeurope.logic.azure.com/workflows/...")
```

### Post

A webhook that forwards the raw server-emitted CloudEvent envelope rather than the lib's narrowed `Notification` view. Use it when a downstream consumer expects the full CloudEvent shape.

```python
aviso.Trigger.post("https://collector.example.org/events")
```

## Tunables

Every trigger accepts the same four tunables, either as keyword arguments to the constructor or as chainable setters:

```python
aviso.Trigger.echo(retries=3, required=False)
aviso.Trigger.echo().retries(3).required(False)
```

- `retries`: number of additional attempts after the first failure. Default `0`.
- `required`: required triggers terminate the watch on failure; optional triggers log a `WARN` and the watch continues. Default `True`.
- `timeout`: meaningful for command and HTTP-based triggers; silently ignored on echo and log.
- `fail_fast`: meaningful for command and HTTP-based triggers; treats deterministic failures (non-zero exit, 4xx HTTP) as terminal. Default `True`.

## When to use which

| Use case | Trigger |
|---|---|
| Eyeball what's arriving | `echo` |
| Audit log of every delivery | `log` |
| Kick off a shell pipeline | `command` |
| Push to a generic HTTP receiver | `webhook` |
| Push to a Microsoft Teams channel | `teams` |
| Forward CloudEvents to a collector | `post` |
| Run arbitrary Python code per notification | (use the iteration loop body, no trigger needed) |

The iteration loop body is the right place for in-process Python logic. Triggers are for declarative durable side effects that should keep working when the calling program exits.
