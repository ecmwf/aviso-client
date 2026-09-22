<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

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
values. Spatial coordinates use latitude first. These spatial examples use the
server's public
[observations schema](https://sites.ecmwf.int/docs/aviso-server/main/practical-examples/point-cloud-filtering.html),
which requires a date, a point cloud, and a payload:

```cpp
const std::string identifier = R"({
  "date": "20260601",
  "point_cloud": [[46, 8], [47, 9]]
})";
std::string response = client.notify_json(
    "observations", identifier, std::string(R"({"source":"stations"})"));
```

A polygon has the same `[[lat, lon], ...]` shape as a point cloud, but needs at
least four pairs with the first pair repeated last. Clouds need no closing
repeat; duplicate points are valid and their order is preserved. Subscribers
filter clouds with `polygon`, not `point_cloud`. The built-in `point` is only a
watch/replay filter for polygon streams, not a provider identifier. Do not quote
arrays inside the JSON object. The `notify_json_async` method provides the same
path for asynchronous calls.

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
  {"event_type": "observations", "identifier": {"date": "20260601", "point_cloud": [[46, 8], [47, 9]]}, "payload": {"source": "stations-a"}},
  {"event_type": "observations", "identifier": {"date": "20260602", "point_cloud": [[48, 10], [49, 11]]}, "payload": {"source": "stations-b"}}
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

[`examples/cpp/basics/02_publish.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/basics/02_publish.cpp)
publishes one notification with a string identifier.
[`examples/cpp/basics/06_publish_polygon.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/basics/06_publish_polygon.cpp)
sends a spatial identifier and a required payload with `notify_json`.
[`examples/cpp/basics/04_publish_many.cpp`](https://github.com/ecmwf/aviso-client/blob/main/examples/cpp/basics/04_publish_many.cpp)
publishes a batch with `notify_many`, with one bad item so you can see a
per-item failure next to two successes. CI compiles and runs all three on every
change, so they never drift from the binding.
