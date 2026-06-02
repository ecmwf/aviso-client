# Blocking verbs

Once you have a `Client`, the blocking verbs publish notifications, read
schemas, and run the admin operations. Each one blocks until the server
responds and throws `aviso::Error` on any failure, so wrap calls in a
`try` / `catch` (see [Overview](./overview.md) for the error shape).

JSON-shaped data crosses the binding as strings: `notify` takes the payload as
a JSON string and returns the server response as a JSON string, and the schema
verbs return JSON strings. Parse them with whatever JSON library your
application already uses.

## Publishing

`notify` publishes one notification. The event type is required; the identifier
is a `std::map<std::string, std::string>` matching the stream's schema, and the
payload is an optional JSON string. It returns the server's response as JSON.

```cpp
std::map<std::string, std::string> identifier = {{"date", "20260101"},
                                                 {"time", "0000"}};
std::string response = client.notify("test_event", identifier);
// With a payload:
std::string with_payload =
    client.notify("test_event", identifier, std::string(R"({"value": 42})"));
```

## Schema discovery

`schema` returns the full catalogue (`GET /api/v1/schema`); `schema_for`
returns one stream's schema (`GET /api/v1/schema/{event_type}`). Both return
JSON strings.

```cpp
std::string catalogue = client.schema();
std::string one = client.schema_for("test_event");
```

## Admin

The admin verbs are operator-only and return nothing on success. `wipe_stream`
removes every notification for one stream, `wipe_all` removes them for every
stream, and `delete_notification` removes a single notification by its
`<event_type>@<sequence>` id.

```cpp
client.wipe_stream("test_event");
client.delete_notification("test_event@42");
client.wipe_all();
```

## A note on threads

These verbs block the calling thread. Do not call them from inside a watch or
async callback (which runs on a runtime thread); doing so throws an
`aviso::Error` with kind `AvisoErrorKind_InvalidUsage` rather than deadlocking.
The watch surface lands in a later release.

## A worked publisher

The [`examples/cpp/publish.cpp`](https://github.com/ecmwf/aviso-client/tree/main/examples/cpp)
example publishes a notification and then reads the stream's schema against a
running server. It is the tested reference for these verbs.
