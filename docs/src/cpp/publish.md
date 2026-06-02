# Publishing

`notify` publishes one notification and returns the server's response as a JSON
string. The event type is required; the identifier is a
`std::map<std::string, std::string>` matching the stream's schema, and the
payload is an optional JSON string. Like every call it throws `aviso::Error` on
failure (see [Overview](./overview.md#how-calls-behave)).

```cpp
std::map<std::string, std::string> identifier = {{"date", "20260101"},
                                                 {"time", "0000"}};
std::string response = client.notify("test_event", identifier);
// With a payload:
std::string with_payload =
    client.notify("test_event", identifier, std::string(R"({"value": 42})"));
```

To see which identifier fields a stream expects, read its schema first with
`schema_for`; see [Operations](./operations.md#schemas).

## A worked publisher

The [`examples/cpp/publish.cpp`](https://github.com/ecmwf/aviso-client/tree/main/examples/cpp)
example publishes a notification and then reads the stream's schema against a
running server. It is the tested reference.
