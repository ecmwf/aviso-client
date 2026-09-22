<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Async

Every blocking verb except `notify_many` has an async form that returns a
`std::future` and runs on a background thread, so several calls can be in
flight at once. The async forms
are also safe to call from inside a listener or async callback, where a blocking
verb would throw `AvisoErrorKind_InvalidUsage`.

The future holds the same value the blocking verb returns: a JSON string for
`notify_async`, `schema_async`, and `schema_for_async`, and nothing
(`std::future<void>`) for `wipe_stream_async`, `wipe_all_async`, and
`delete_notification_async`. Calling `.get()` blocks for the result and rethrows
any `aviso::Error`.

```cpp
std::future<std::string> published =
    client.notify_async("test_event", identifier);
std::future<std::string> catalogue = client.schema_async();

std::cout << published.get() << '\n';  // rethrows aviso::Error on failure
std::cout << catalogue.get() << '\n';
```

## When to reach for them

For publishing a batch, `notify_many` already sends concurrently and reports
per-item failures without throwing; see [Publishing](./publish.md). It has no
async form because it does not need one. The async verbs earn their keep when
the requests are not all publishes, or when you have work to do between
starting them and collecting. Inside a callback they are the only option,
since a blocking verb throws there.

The pattern for many requests is: start them all, keep the futures, then
`get()` each one. A failed request throws from its own `get()` and the others
are unaffected, so wrap each `get()` in a `try` if one failure should not stop
you collecting the rest.

## Complete examples

[`examples/cpp/async/01_basic.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/async/01_basic.cpp)
puts two requests in flight at once and times them.
[`examples/cpp/async/02_fan_out.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/async/02_fan_out.cpp)
starts ten publishes and a schema lookup, then collects them all in about one
round trip.
