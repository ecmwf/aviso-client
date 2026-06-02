# Watching

A watch streams notifications to your code as they arrive. Unlike the blocking
verbs, it is callback-driven: you subclass `aviso::NotificationHandler`, start
the watch, and the library calls you back on a background thread until you stop.

## The handler

Subclass `NotificationHandler` and override `on_notification`. Return `false` to
ask the watch to stop. Override `on_end` to learn when the watch finishes and
whether it failed.

```cpp
class Handler : public aviso::NotificationHandler {
 public:
  bool on_notification(const aviso::Notification& n) override {
    std::cout << n.event_type() << " #" << n.sequence() << " "
              << n.identifier_json() << " " << n.payload_json() << '\n';
    return true;  // keep going; return false to stop
  }
  void on_end(const std::optional<aviso::ErrorInfo>& error) override {
    if (error) {
      std::cerr << "watch failed: " << error->message << '\n';
    }
  }
};
```

The callbacks run on a runtime thread, so they must be thread-safe and must not
make blocking aviso calls (those throw an `aviso::Error` whose `error().kind` is
`AvisoErrorKind_InvalidUsage`; see [Overview](./overview.md#how-calls-behave)).
The notification passed to
`on_notification` is a borrowed view valid only for that call; its accessors
return owned `std::string`s, so copy out anything you keep. There is no
per-notification request id; the `sequence` is the stream cursor.

## Starting and stopping

Build a `WatchRequest`, then call `client.watch`. It returns an RAII `Watch`
whose destructor stops the watch and waits for it to finish, so the handler
outlives the watch automatically. The handler reference you pass must outlive
the returned `Watch`.

```cpp
Handler handler;
aviso::WatchRequest request("test_event");
aviso::Watch watch = client.watch(request, handler);

// Block until the watch ends (the handler returned false, the stream ended, or
// it failed). Or just let `watch` go out of scope to stop and wait.
watch.wait();
```

`watch.stop()` requests a graceful stop at any time; it is nonblocking and safe
to call from inside the handler. `watch.wait()` blocks until the handler's
`on_end` has returned (do not call it from inside a callback).

## Resuming and replaying

By default a watch is live. The request builder can resume after a sequence or a
date, or switch to replay-only (which reads history and then ends rather than
going live):

```cpp
aviso::WatchRequest("test_event").watch_from_sequence(42);  // resume then live
aviso::WatchRequest("test_event").replay_from_sequence(0);  // history then end
aviso::WatchRequest("test_event").watch_from_date("2026-01-01T00:00:00Z");
```

## Filtering

`filter_json` narrows the stream to notifications whose identifier matches a
JSON object (an exact value per key, or a server-defined rule object):

```cpp
aviso::WatchRequest("test_event").filter_json(R"({"date":"20260101"})");
```

## A complete example

[`examples/cpp/watch.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/watch.cpp)
starts a watch and, from another thread on the same client, publishes a few
notifications so the live watch receives them. It is the one-shared-client
pattern: one client, a watch on a runtime thread, and publishes from your own
threads.
