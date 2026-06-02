# Triggers

A trigger is a per-notification side effect attached to a watch. When the watch
processes a notification, its triggers fire before the notification reaches your
handler. Build a trigger with a factory, tune it with the chainable setters, and
attach it to a `WatchRequest` with `add_trigger`.

## Kinds

- `Trigger::echo()` writes each notification as JSON to standard output.
- `Trigger::log(path)` appends each notification as JSON to a file.
- `Trigger::command(cmd)` runs `/bin/sh -c <cmd>` per notification, with the
  notification's fields exposed as `AVISO_*` environment variables (Unix only).
- `Trigger::webhook(url)` sends an HTTP request per notification, with a
  settable method, headers, and body template.
- `Trigger::teams(url)` posts an Adaptive Card to a Teams Workflows webhook.
- `Trigger::post(url)` HTTP POSTs the raw `CloudEvent` envelope.

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

## Reliability

Every trigger has `retries` (default 0), `required` (default true), a `timeout`,
and `fail_fast` (default true). A required trigger that still fails after its
retries ends the watch with an `aviso::Error` whose `error().kind` is
`AvisoErrorKind_Trigger`; `error().trigger_kind` names the trigger and
`error().error_kind` names the failure. An optional trigger (`required(false)`)
that fails is logged and the watch continues.

## A complete example

[`examples/cpp/trigger.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/trigger.cpp)
attaches a log trigger to a watch and, once the watch has received a few
notifications, prints the file the trigger wrote. Publish to the stream from
another terminal to drive it.
