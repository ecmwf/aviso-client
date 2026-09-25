<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Triggers

A trigger runs an action automatically for each matching notification, such as
appending to a file or sending an HTTP request. Add triggers to
`client.listen()` to run them before Python receives the notification in your
loop. To run your own Python analysis in the loop, you do not need a trigger.

Triggers run in your client process. They do not keep listening or start new
actions after your Python program exits.

## A complete listener with triggers

Use the installation and environment setup from the
[quickstart](./quickstart.md#set-the-environment), including `AVISO_BASE_URL`
and credentials for `pyaviso.Env()`. For an anonymous server, pass
`auth=pyaviso.Anonymous()` instead.

This uses the quickstart's `mars` schema: `class` is a required choice of `od`
or `rd`; `step` is an integer, optional in filters. Leaving it out selects all
steps. Providers must supply both identifier fields when publishing. The
optional payload can carry a file location. Check
[your server's schema](./quickstart.md#what-is-on-your-server) if it differs.

Save this as `listen.py` and run `python listen.py` in that terminal:

```python
import os

import pyaviso
from pyaviso import Trigger

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
with client.listen(
    "mars",
    filter={"class": "od"},
    triggers=[Trigger.log('mars.log')],
) as notifications:
    for notification in notifications:
        print(notification)
```

The script waits for new `mars` notifications with `class=od`. For each one, it
appends a JSON line to `mars.log` in your current directory, then prints the
notification. A notification with `class=rd` causes neither action. Press Ctrl+C
to stop; the `with` block closes the listener.

Run this from a directory you can write to. If the log file cannot be written,
the listener stops with an error by default.

The two outputs contain different views of the same notification.
`print(notification)` shows the original CloudEvent as indented JSON, including
its `specversion`, `type` and `data`. The log contains a smaller JSON object on
one line, with fields such as `event_type`, `sequence`, `identifier` and
`payload`. A payload such as `{"location": "file:///data/forecast.grib"}` is a
reference to a file; the log trigger does not copy that file.

## When to use which

| You want to | Use | Details |
|---|---|---|
| Print notifications automatically | `Trigger.echo()` | [Echo guide](../triggers/echo.md) |
| Append notifications to a file | `Trigger.log()` | [Log guide](../triggers/log.md) |
| Run a shell command | `Trigger.command()` | [Command guide](../triggers/command.md) |
| Send an HTTP request with your chosen body | `Trigger.webhook()` | [Webhook guide](../triggers/webhook.md) |
| Send a card to a Teams workflow webhook | `Trigger.teams()` | [Teams guide](../triggers/teams.md) |
| Forward the original CloudEvent | `Trigger.post()` | [Post guide](../triggers/post.md) |
| Run your own Python function | `Trigger.function()` | [Below](#function) |

## The six kinds

Each fragment below replaces the `triggers=[...],` line in `listen.py`. Keep the
imports, client setup and loop. Run `python listen.py` again after each change.
For HTTP examples, set the named environment variable to a receiver URL you
control before running the script.

### Echo

Print each matching notification, using the same smaller view as the log
trigger:

```python
    triggers=[Trigger.echo()],
```

In a terminal, echo prints a heading and indented JSON. When redirected or
piped, it prints one compact JSON line. The loop still prints the CloudEvent
afterwards, so you see both views.

### Log

To keep a log and also echo each notification, put both actions in the list:

```python
    triggers=[Trigger.log("mars.log"), Trigger.echo()],
```

Actions run in list order before the loop receives that notification. The log
file opens on the first matching notification and appends to existing content.

### Command (Unix only)

Append each forecast step to `steps.txt` in your current directory:

```python
    triggers=[
        Trigger.command('printf "%s\\n" "$AVISO_IDENTIFIER_STEP" >> steps.txt')
    ],
```

This runs through `/bin/sh -c`. The client automatically supplies
`AVISO_IDENTIFIER_CLASS` and `AVISO_IDENTIFIER_STEP` for this schema, plus
`AVISO_EVENT_TYPE`, `AVISO_SEQUENCE`, `AVISO_PAYLOAD_JSON` and
`AVISO_NOTIFICATION_JSON`. Quote shell variable expansions as above. Prefer
these variables to inserting notification text into shell source with templates.
Command stdout is captured, so write to a file when you want to keep the output.
On non-Unix builds, the constructor raises `pyaviso.ConfigError`.

### Webhook

Set `FORECAST_WEBHOOK_URL` to your HTTP receiver's URL. Send it a JSON body with
the forecast step:

```python
    triggers=[
        Trigger.webhook(
            os.environ["FORECAST_WEBHOOK_URL"],
            body_template='{"step": {{ notification.identifier.step }}}',
        )
    ],
```

The default method is `POST`. Here the template writes `step` as a JSON number.
Without `body_template`, the body is the smaller notification view used by log
and echo. URL, header values and body templates can read notification fields
and environment variables. See the
[template guide](../triggers/template-engine.md) for syntax and escaping rules.

### Teams

Set `FORECAST_TEAMS_URL` to your Teams workflow webhook URL:

```python
    triggers=[Trigger.teams(os.environ["FORECAST_TEAMS_URL"])],
```

The client builds an Adaptive Card from the notification and sends it by HTTP
POST. Configure the receiving workflow as described in the
[Teams guide](../triggers/teams.md).

### Post

Set `FORECAST_COLLECTOR_URL` to your CloudEvent receiver's URL:

```python
    triggers=[Trigger.post(os.environ["FORECAST_COLLECTOR_URL"])],
```

This sends the original server CloudEvent as JSON by HTTP POST. It preserves the
envelope fields shown by `print(notification)`, including CloudEvent extensions.
Use it when the receiver needs those fields rather than the smaller default
webhook body.

## Function

`Trigger.function(func)` calls a Python function of yours with each
notification. It is the way to run your own code without writing the loop:
`run()` reads the stream to the end and closes it.

```python
import pyaviso
from pyaviso import Trigger


def save(notification):
    print("save", notification.identifier)


client = pyaviso.AvisoClient()
try:
    client.listen(
        "mars",
        filter={"class": "od"},
        triggers=[Trigger.log("mars.log"), Trigger.function(save)],
    ).run()
except KeyboardInterrupt:
    print("Stopped listening")
```

When an `od` notification is published, a line is appended to `mars.log` and
the script prints:

```text
save {'class': 'od', 'step': '12'}
```

A function trigger differs from the six built-in kinds in where it runs. The
built-in kinds run inside the library, before the notification reaches
Python. A function runs in the application, in the thread that reads the
notification (or on its event loop with `AsyncAvisoClient`), when the
notification reaches it. Consequently:

- The built-in triggers for a notification always run before its functions,
  regardless of their order in the list.
- Functions run one notification at a time, in arrival order. They need no
  synchronisation and may call blocking code. A slow function delays the
  listener; no notification is lost, because the client reads from the server
  more slowly. On `AsyncAvisoClient`, a blocking call in a function blocks the
  event loop, and with it every listener; use `async def` and awaitable calls
  there.
- With `AsyncAvisoClient`, a function may be defined with `async def`; it is
  awaited. `AvisoClient.listen()` rejects such a function with a `TypeError`
  when called, because nothing would await it.
- The notification has already been received when a function runs. Unlike a
  failed built-in trigger, a failed function therefore never holds back the
  [saved position](./state-and-resume.md), whether or not the loop continues:
  the notification may already be recorded as received, and is then not
  delivered again after a restart. Record completed work separately when
  every notification must be processed.

The function receives the notification as its only argument. Bind further
arguments with `functools.partial(save, folder="/data")` or a `lambda`.
Several functions run in the order listed.

To use a different function for each of several listeners, see
[Multiple listeners](./listen-many.md#function-triggers-per-listener).

## Tunables

By default, every trigger is required and has no retries. If a required action
fails, listening stops with `pyaviso.TriggerError`; that notification does not
reach the Python loop. Earlier actions are not undone. Set `required=False` for
an action whose failure should emit a warning and let later actions and the
Python loop continue.

For example, replace the trigger list with this optional webhook, allowing two
extra attempts and a five-second timeout per request:

```python
    triggers=[
        Trigger.webhook(
            os.environ["FORECAST_WEBHOOK_URL"],
            retries=2,
            required=False,
            timeout=5.0,
        )
    ],
```

- `retries=0` means one attempt. `retries=2` allows up to three attempts.
- `fail_fast=True` is the default for command and HTTP triggers. A non-zero
  command exit, an HTTP 4xx response, an invalid request or a template error
  fails immediately, bypassing retries. HTTP 5xx responses, connection errors
  and timeouts can retry. Set `fail_fast=False` to retry those immediate
  failures too, within the configured retry count.
- HTTP triggers (`webhook`, `teams`, `post`) default to a 30-second timeout per
  request. Commands have no timeout by default; use `timeout` to set one in
  seconds. Echo and log do not accept a timeout keyword. Their `.timeout()`
  setter is ignored, as is `.fail_fast()`.
- `Trigger.function` takes `retries`, `required` and `label`. A function that
  raises counts as a failure: it is called again up to `retries` times, then a
  required one raises `TriggerError` (with `trigger_kind == "function"` and the
  original exception as its `__cause__`) and the functions after it are not
  called for that notification; an optional one is logged and skipped.
  `timeout` and `fail_fast` do not apply to functions and raise `ValueError`:
  Python code cannot be interrupted safely.
- A plain function that returns an awaitable is a programming error, not a
  failure of one notification: it raises `TypeError`, whatever `required` and
  `on_error` say.

The [builder pattern](./builder-pattern.md) covers chainable setters and reusing
triggers. See [error handling](./error-handling.md) for catching client errors.

## With `AsyncAvisoClient`

Pass the same `triggers=[...]` list to `AsyncAvisoClient.listen()`. Actions
still run before the notification reaches your `async for` loop, and
`Trigger.function` may be given an `async def` function, which is awaited. Use
`await iterator.run()` in place of `.run()`. See [Async](./async.md) for a
complete listener.
