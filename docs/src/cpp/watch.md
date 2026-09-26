<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Listening

A listener streams notifications to your code as they arrive. In C++ you start
one with `client.watch`, passing a `WatchRequest` that says what to listen to
and a handler that says what to do with each notification. The library calls
your handler on a background thread until the listener ends, whether because
you stopped it or because it failed.

## The handler

Subclass `aviso::NotificationHandler` and override two methods.
`on_notification` runs once per notification and returns whether to keep
going. `on_end` runs once, after the last notification, and tells you whether
the listener stopped because you asked or because it failed.

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

Do not leave `on_end` empty. It is the only place a failed watch is reported.
An exception thrown out of `on_notification` also ends up there: it stops the
watch, and `on_end` receives an `AvisoErrorKind_Internal` error whose message
names the exception.
`wait()` returns normally whether the watch ended because your handler said so
or because the connection was refused, so a handler that ignores `on_end`
turns every failure into a quiet exit. The examples keep a small base class in
`common.hpp` that stores the error, and a `finish()` helper that turns it into
the exit code.

The callbacks run on a runtime thread, so the handler must be thread-safe.
They must not make blocking aviso calls: a blocking call from inside a callback
throws an `aviso::Error` of kind `AvisoErrorKind_InvalidUsage`, and the async
verbs are the way to make requests from there (see [Async](./async.md)). The
`Notification` you receive is a borrowed view, valid only for that call. Its
accessors return owned `std::string`s, so copy out anything you want to keep.

## Starting and stopping

Build a `WatchRequest`, then call `client.watch`. It returns a `Watch` whose
destructor stops the listener and waits for it, so you cannot leave one
running by accident. The handler you pass must outlive the `Watch`.

```cpp
Handler handler;
aviso::WatchRequest request("test_event");
aviso::Watch watch = client.watch(request, handler);

// Block until listening ends: the handler returned false, the stream ended,
// or it failed. Or just let `watch` go out of scope to stop and wait.
watch.wait();
```

`watch.stop()` asks for a graceful stop. It returns at once and is safe from
any thread, including from inside the handler. After a `stop()`, `on_end` runs
with no error, so a stopped watch looks the same as one whose handler returned
`false`.
`watch.wait()` blocks until `on_end` has returned, so do not call it from
inside a callback.

A signal handler may not call `stop()` itself, since almost nothing is allowed
inside one. The pattern that works is to set a flag in the signal handler and
have a small thread call `stop()` when the flag turns.

[`examples/cpp/basics/03_listen.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/basics/03_listen.cpp)
is this in full: it prints each notification and stops itself after three.
[`examples/cpp/resilience/04_stop_from_outside.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/resilience/04_stop_from_outside.cpp)
runs until Ctrl+C or a timer, and stops the way a real listener would.

## Resuming where you left off

Every notification carries a sequence number. If your process stops, the last
sequence you handled is all you need to carry on without a gap: ask for
everything after it, then stay live. The server replays what you missed first
and switches to live delivery on the same connection.

```cpp
aviso::WatchRequest request("test_event");
request.watch_from_sequence(last_handled);  // replay after this, then live
```

Where you keep `last_handled` is up to you. A file is enough for a single
process; record it in `on_notification` **after** handling, not before, so a
crash mid-handler replays that notification rather than skipping it. That
gives you at-least-once delivery, which is what you want for anything that
matters.

`watch_from_date` does the same with a timestamp instead of a sequence.

[`examples/cpp/resilience/01_resume_from_sequence.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/resilience/01_resume_from_sequence.cpp)
keeps the position in a file. Run it, stop it, publish a few notifications,
run it again: the missed ones arrive first.

## Reading history only

Sometimes you want a batch, not a feed. `replay_from_sequence` and
`replay_from_date` deliver what the server has kept and then end the watch by
themselves, without waiting for anything new. The handler can return `true`
throughout; `wait()` still returns when history runs out.

```cpp
aviso::WatchRequest request("test_event");
request.replay_from_date("2026-01-01T00:00:00Z");  // history, then end
```

A server caps how much one replay may return. When you hit the cap the watch
ends with an error of kind `AvisoErrorKind_HistoryGap` whose message names the
limit, and the notifications you did get are complete and in order up to the
last one. Ask again with `replay_from_sequence` from that last sequence to get
the rest, and repeat until the watch ends without an error.

[`examples/cpp/resilience/02_replay_only.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/resilience/02_replay_only.cpp)
does exactly that: it reads everything since a date in as many batches as the
cap requires, then exits with a count.

## Filtering

`filter_json` narrows the stream to notifications whose identifier matches a
JSON object: an exact value per key, or a server-defined rule object.

```cpp
aviso::WatchRequest("test_event").filter_json(R"({"date":"20260101"})");
```

Fields you leave out match anything. The filter is applied by the server, so
notifications that do not match never reach your process.
[`examples/cpp/basics/05_filter.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/basics/05_filter.cpp)
listens for one date and lets everything else go past.

### Numeric and enum constraints

With the schema and seeds from the
[weather tutorial](../cli/publish-and-listen.md#weather-constraints), use this
request with the `Handler` above and a `client` built as in
[A first call](./overview.md#a-first-call), using your test server's address and
credentials. Replay delivers B and C, then ends; inspect their labels in
`n.payload_json()`.

```cpp
aviso::WatchRequest request("weather");
request.replay_from_sequence(0).filter_json(R"({
  "date": "20260913",
  "severity": {"gte": 5},
  "anomaly": {"between": [40, 50]},
  "region": {"in": ["north", "south"]}
})");
Handler handler;
auto watch = client.watch(request, handler);
watch.wait();
```

For live delivery, omit `replay_from_sequence(0)` and start before publishing.
The raw string contains JSON objects with numeric operands, not quoted JSON
objects. See [Filters](../concepts/filters.md#constraint-filters) for supported
operators and schema requirements.
