# Async

Every blocking verb has an async form that returns a `std::future` and runs on
a background thread, so several calls can be in flight at once. The async forms
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

## A complete example

[`examples/cpp/async.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/async.cpp)
fires two async verbs at once and waits on their futures.
