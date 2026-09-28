<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Triggers

A trigger is a per-notification side effect attached to a listener. When it
processes a notification, its triggers fire before the notification reaches your
handler. Build a trigger with a factory, tune it with the chainable setters, and
attach it to a `WatchRequest` with `add_trigger`.

## Kinds

- `Trigger::echo()` writes each notification as one line of JSON to standard
  output. The one to try first, because it shows you exactly what every other
  trigger receives.
- `Trigger::log(path)` appends each notification as JSON to a file, across
  runs. A durable record with no file handling of your own.
- `Trigger::command(cmd)` runs `/bin/sh -c <cmd>` per notification (Unix
  only). The notification arrives as environment variables:
  `AVISO_EVENT_TYPE`, `AVISO_SEQUENCE`, one `AVISO_IDENTIFIER_<FIELD>` per
  identifier field, and `AVISO_NOTIFICATION_JSON` for the whole thing. The
  command's own stdout is captured and dropped, so write to a file if you want
  to see output.
- `Trigger::webhook(url)` sends an HTTP request per notification, with a
  settable method, headers, and a body template that can name notification
  fields, such as `{{ notification.identifier.date }}`.
- `Trigger::teams(url)` posts an Adaptive Card to a Teams Workflows webhook.
- `Trigger::post(url)` HTTP POSTs the raw `CloudEvent` envelope.

Triggers fire whether or not your handler does anything with the
notification. A handler that only counts, paired with a trigger that does the
real work, is a perfectly good listener.

## Attaching

`add_trigger` consumes the trigger. A factory result attaches directly:

```cpp
aviso::WatchRequest request("test_event");
request.add_trigger(aviso::Trigger::log("/var/log/aviso/notifications.log"));
aviso::Watch watch = client.watch(request, handler);
```

A trigger is move-only, so to tune one with the setters, build it as a named
value and move it in:

```cpp
aviso::Trigger hook = aviso::Trigger::webhook("https://example.com/hook");
hook.method(aviso::HttpMethod::Post)
    .header("Authorization", "Bearer " + token)
    .retries(3)
    .timeout_secs(10);
request.add_trigger(std::move(hook));
```

After `add_trigger`, `hook` is used up. Calling a setter on it, or attaching
it again, throws an `aviso::Error` of kind `AvisoErrorKind_InvalidUsage`.

## Reliability

Every trigger has `retries` (default 0), `required` (default true), a
`timeout_secs`, and `fail_fast` (default true). Set `label` to name the trigger
in error messages when you attach more than one.

`required` is the setting that decides what a failure means. A required
trigger that still fails after its retries ends the listener: `on_end`
receives an `aviso::ErrorInfo` of kind `AvisoErrorKind_Trigger`, with
`trigger_kind` naming the trigger and `error_kind` naming the failure. An
optional trigger (`required(false)`) that fails is skipped and the listener
carries on. Nothing tells your code about it: there is no callback and no log
you can read from C++, and the notification reaches your handler as if the
trigger had worked. If you need to know when a trigger fails, make it required,
or have the trigger leave its own trace. Use required for the side effect the
listener exists to perform, and optional for the ones that are nice to have.

Triggers run in the order you added them, for every notification, before your
handler sees it.

## Complete examples

Each file in
[`triggers/`](https://github.com/ecmwf/aviso-client/tree/main/examples/cpp/triggers)
covers one kind, and the last one is a composition:

- `01_echo.cpp` and `02_log.cpp` are the two simplest, with a handler that
  only counts.
- `03_command.cpp` runs a shell command that appends the `AVISO_*` variables
  to a file, then prints the file.
- `04_webhook.cpp` POSTs a templated body to a URL. It opens a tiny receiver
  on a loopback port so it runs without one, and prints what arrived.
- `05_multiple.cpp` attaches a required log, an optional echo, and an optional
  command that always fails, so you can see the failure leave the listener
  running. Change it to `required(true)` and the listener ends on the first
  notification instead.

Every one stops itself after three notifications; publish from another
terminal to drive them.
