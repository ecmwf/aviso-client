# Operations

The schema lookups and the admin calls. Each one either succeeds or throws
`aviso::Error` (see [Overview](./overview.md#how-calls-behave)).

## Schemas

`schema` returns the full catalogue (`GET /api/v1/schema`); `schema_for`
returns one stream's schema (`GET /api/v1/schema/{event_type}`). Both return
JSON strings.

```cpp
std::string catalogue = client.schema();
std::string one = client.schema_for("test_event");
```

## Admin

The admin calls are operator-only and return nothing on success. `wipe_stream`
removes every notification for one stream, `wipe_all` removes them for every
stream, and `delete_notification` removes a single notification by its
`<event_type>@<sequence>` id.

```cpp
client.wipe_stream("test_event");
client.delete_notification("test_event@42");
client.wipe_all();
```
