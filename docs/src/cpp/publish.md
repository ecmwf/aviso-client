# Publishing

`notify` publishes one notification and returns the server's response as a JSON
string. Its map overload is convenient for string-only identifiers. The payload
is an optional JSON string. Like every call it throws `aviso::Error` on failure
(see [Overview](./overview.md#how-calls-behave)).

```cpp
std::map<std::string, std::string> identifier = {{"date", "20260101"},
                                                 {"time", "0000"}};
std::string response = client.notify("test_event", identifier);
// With a payload:
std::string with_payload =
    client.notify("test_event", identifier, std::string(R"({"value": 42})"));
```

Use `notify_json` when an identifier contains arrays, objects, or other JSON
values. Spatial coordinates use latitude first:

```cpp
const std::string identifier = R"({
  "point": [46, 8],
  "point_cloud": [[46, 8], [47, 9]]
})";
std::string response = client.notify_json("observations", identifier);
```

A polygon has the same `[[lat, lon], ...]` shape as a point cloud. Its points
describe a ring, and the first point is repeated when the schema requires a
closed polygon. Do not quote the arrays inside the JSON object. The
`notify_json_async` method provides the same path for asynchronous calls.

To see which identifier fields a stream expects, read its schema first with
`schema_for`; see [Operations](./operations.md#schemas).

## Publishing many at once

`notify_many` sends a whole batch concurrently and returns a JSON array with one
entry per input, in order. Over a single HTTP/2 connection a batch that would
take many sequential round-trips finishes in roughly one. You pass the batch as
a JSON array string of `{event_type, identifier?, payload?}` objects, built with
whatever JSON you like, and an optional `max_concurrency` (0 selects a default).

```cpp
const std::string notifications = R"([
  {"event_type": "observations", "identifier": {"point_cloud": [[46, 8], [47, 9]]}},
  {"event_type": "observations", "identifier": {"point_cloud": [[48, 10], [49, 11]]}}
])";
std::string results = client.notify_many(notifications);
```

The batch is not atomic. A per-item failure comes back as an `"error"` entry in
the array rather than throwing, so one bad notification does not sink the rest;
only a malformed array (not valid JSON, or an item missing `event_type`) throws.
Each entry is `{"index", "status", "response"}` on success or
`{"index", "status", "error"}` on failure, where the error carries `kind`,
`http_status`, `message`, and `request_id`.

## A complete example

[`examples/cpp/publish.cpp`](https://github.com/ecmwf/aviso-client/tree/main/examples/cpp)
publishes a notification and then reads back the stream's schema, and
[`examples/cpp/publish_many.cpp`](https://github.com/ecmwf/aviso-client/tree/main/examples/cpp)
publishes a batch with `notify_many`. CI compiles and runs both on every change,
so they never drift from the binding.
