# Rust library guide

How to use the `aviso` crate from your own Rust code.

## Adding aviso to your project

```toml
[dependencies]
aviso = "2.0"
tokio = { version = "1.45", features = ["macros", "rt-multi-thread"] }
serde_json = "1.0"
```

aviso is async and runs on tokio.

## Building a client

```rust,ignore
use aviso::AvisoClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("https://aviso.example")
        .build()?;

    println!("client base url = {}", client.base_url());
    Ok(())
}
```

The client is `Clone`. Cloned handles share the same HTTP connection pool and
the same authentication provider, so you can hand copies to multiple tasks
without paying for extra sockets.

The builder normalises the base URL: a trailing slash is added if missing; a
path prefix (`https://gw.example/aviso`) is preserved so the client works behind
a reverse proxy. Endpoint paths are joined relatively, never absolutely; the
absolute form would strip the proxy prefix.

## Authentication

```rust,ignore
use std::sync::Arc;
use aviso::{AvisoClient, auth::Bearer};

let auth = Arc::new(Bearer::new("opaque-or-jwt-token")?);
let client = AvisoClient::builder()
    .base_url("https://aviso.example")
    .auth(auth)
    .build()?;
```

Five built-in providers are available: `Basic`, `Bearer`, `Env`, `ConfigFile`,
and `Chain`. The page on
[authentication providers](../concepts/auth-providers.md) covers when to use
each one. The full API is at
[`aviso::auth`](https://docs.rs/aviso/latest/aviso/auth/).

For custom providers (OAuth, OIDC, AWS SigV4, ...), implement the `AuthProvider`
trait. Always call `HeaderValue::set_sensitive(true)` on the value you return;
that is what makes downstream loggers redact it.

## Publishing a notification

```rust,ignore
use std::collections::BTreeMap;
use std::sync::Arc;

use aviso::{AvisoClient, NotificationRequest, auth::Bearer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("https://aviso.example")
        .auth(Arc::new(Bearer::new("opaque-jwt")?))
        .build()?;

    let mut identifier = BTreeMap::new();
    identifier.insert("date".into(), serde_json::json!("20260601"));
    identifier.insert(
        "point_cloud".into(),
        serde_json::json!([[46.0, 8.0], [47.0, 9.0]]),
    );

    let request = NotificationRequest::new("observations")
        .with_identifier(identifier)
        .with_payload(serde_json::json!({ "location": "s3://bucket/path" }));

    let response = client.notify(&request).await?;
    println!(
        "published: request_id={}, processed_at={}",
        response.request_id, response.processed_at
    );
    Ok(())
}
```

`NotificationRequest::identifier` is a
`BTreeMap<String, serde_json::Value>`. Points use `[lat, lon]`; polygons and
point clouds use `[[lat, lon], ...]`. Received `Notification` values use the
same map type, so structured identifiers retain their array shape.

This spatial example uses the server's public
[observations schema](https://sites.ecmwf.int/docs/aviso-server/main/practical-examples/point-cloud-filtering.html).
Polygons need at least four pairs with the first repeated last. Clouds need no
closing repeat; duplicate points are valid and their order is preserved.
Subscribers filter clouds with `polygon`, not `point_cloud`. The built-in
`point` is only a watch/replay filter for polygon streams.

`with_identifier` accepts `BTreeMap<String, serde_json::Value>`. For a
string-only map, use `with_string_identifier`; it converts each entry to a JSON
string. The public `identifier` fields themselves are JSON-valued. Code that
reads `Notification.identifier` must therefore handle `serde_json::Value`
rather than assuming every received value is a `String`.

On a 401, the client calls `AuthProvider::refresh` and retries once. A second
401 is surfaced as `ClientError::Http`.

`notify` does not auto-retry on a transport error after the request body has
been sent (the server may have already processed it; a blind retry would risk a
duplicate).

## Listening for notifications

Two surfaces, one supervisor underneath.

### Stream surface

```rust,ignore
use std::collections::BTreeMap;
use aviso::{watch::WatchRequest, AvisoClient};

let client = AvisoClient::builder().base_url("https://aviso.example").build()?;

let mut filter = BTreeMap::new();
filter.insert("class".to_string(), serde_json::json!("od"));

let mut stream = client.watch(WatchRequest::watch("mars").with_filter(filter))?;

while let Some(item) = stream.recv().await {
    let notification = item?;
    println!("seq {}: {}", notification.sequence, notification.payload);
}
```

The stream is an async `Stream<Item = Result<Notification, ClientError>>`. Drop
it to cancel.

The filter must include every identifier the event type's schema marks
`required: true`. Omitting one returns
`400 Required field '<name>' missing for watch operation`. Run
`aviso schema get <TYPE>` to see which fields are required.

### Numeric and enum constraints

Use JSON objects in the filter map. With the `client` above connected to the
test server from the
[weather tutorial](../cli/publish-and-listen.md#weather-constraints), this
replays the seeded records B and C and then ends:

```rust,ignore
use std::collections::BTreeMap;
use aviso::watch::WatchRequest;
use serde_json::json;

let filter = BTreeMap::from([
    ("date".into(), json!("20260913")),
    ("severity".into(), json!({"gte": 5})),
    ("anomaly".into(), json!({"between": [40, 50]})),
    ("region".into(), json!({"in": ["north", "south"]})),
]);
let request = WatchRequest::replay_only(
    "weather",
    aviso::watch::ResumeStart::AfterSequence(0),
)
.with_filter(filter);
let mut stream = client.watch(request)?;
while let Some(item) = stream.recv().await {
    println!("{}", item?.payload["id"]);
}
```

For live delivery, use `WatchRequest::watch("weather")` and start before
publishing. All bindings use the same
[constraint rules](../concepts/filters.md#constraint-filters); the map selects
identifiers, not payload contents.

### Callback surface

```rust,ignore
use std::collections::BTreeMap;

let mut filter = BTreeMap::new();
filter.insert("class".to_string(), serde_json::json!("od"));

client
    .watch_with_handler(
        WatchRequest::watch("mars").with_filter(filter),
        |notification| async move {
            println!("got sequence {}", notification.sequence);
            Ok(())
        },
    )
    .await?;
```

Same supervisor, same behaviour. Pick the shape that fits your code.

## Building a watch request

Three constructors prevent invalid combinations:

```rust,ignore
use std::collections::BTreeMap;
use serde_json::json;
use aviso::watch::{ResumeStart, WatchRequest};

// Live: stream new notifications as they arrive.
let live = WatchRequest::watch("mars");

// Historical then live: replay after sequence 41, then keep going.
let historical = WatchRequest::watch_from("mars", ResumeStart::AfterSequence(41));

// Replay only: replay from a date, then close cleanly.
let replay = WatchRequest::replay_only("mars", ResumeStart::Date("2026-01-01T00:00:00Z".into()));

println!("{live:?}\n{historical:?}\n{replay:?}");

// Add a filter. Values are JSON, so spatial filters fit too.
let mut filter = BTreeMap::new();
filter.insert("date".to_string(), json!("20260601"));
filter.insert("polygon".to_string(),
              json!([[46.0, 8.0], [46.0, 9.0], [47.0, 9.0], [46.0, 8.0]]));
let req = WatchRequest::watch("observations").with_filter(filter);
```

`ResumeStart::AfterSequence(n)` reads as "I already have everything up to n;
give me n+1 onward". The supervisor sends `from_id = (n + 1).to_string()` on the
wire.

## Triggers from the library

```rust,ignore
use std::time::Duration;
use aviso::watch::{HttpMethod, Trigger, WatchRequest};

let req = WatchRequest::watch("mars")
    .with_triggers(vec![
        Trigger::echo(),
        Trigger::log("/var/log/aviso/mars.log"),
        Trigger::command("./on-event.sh {{ notification.event_type }}")
            .timeout(Duration::from_secs(30))
            .retries(2),
        Trigger::webhook("https://hooks.example/notify")
            .method(HttpMethod::Post)
            .header("Authorization", "Bearer {{ env.HOOK_TOKEN }}")
            .body_template(r#"{"seq": {{ notification.sequence }}}"#),
    ]);
let mut stream = client.watch(req)?;
```

Each trigger has the same four tunables: `retries`, `required`, `timeout`,
`fail_fast`. For YAML equivalents and the trigger contracts, see
[Triggers overview](../triggers/overview.md).

## State store: surviving restarts

```rust,ignore
use std::sync::Arc;
use aviso::state::{JsonFileStore, StateStore};
use std::path::PathBuf;

let path = PathBuf::from("/var/lib/aviso/state.json");
let store: Arc<dyn StateStore> = Arc::new(JsonFileStore::open(&path).await?);

let client = AvisoClient::builder()
    .base_url("https://aviso.example")
    .state_store(store)
    .build()?;
```

When a store is configured:

- At watch start, if your `WatchRequest` has no explicit resume position, the
  supervisor reads the stored checkpoint and resumes from there.
- After each successful notification dispatch, the supervisor commits the
  previous notification's sequence before letting the consumer pull the next.
- The user-facing contract is "pulling item N+1 implies item N is durable".

`MemoryStore` is the in-process equivalent: useful for tests and short-lived
processes.

For the on-disk format and edit safety, see
[State file](../reference/state-file.md).

## Reading the schema

```rust,ignore
let catalog = client.schema().await?;
for name in &catalog.event_types {
    println!("{name}");
}

let one = client.schema_for("mars").await?;
println!("identifier rules: {:?}", one.schema.identifier);
```

aviso does not validate notifications against schemas. These methods are for
discovery.

## Errors

`ClientError` variants you will see most:

| Variant | When |
|---|---|
| `Transport` | DNS, connect, TLS, or partial body before any response. |
| `Http { status, body, request_id }` | Any non-success status. The `request_id` is the server's correlation id; quote it when filing issues. |
| `Decode` | The body did not deserialise as expected (server contract drift). |
| `Auth` | The auth provider failed to produce a header, or `refresh` itself failed. |
| `TriggerFailed { kind, source }` | A required trigger failed after all retries. |
| `StateStore` | A configured `StateStore::put` or `get` failed. Terminal: continuing would silently violate at-least-once delivery. |
| `HistoryGap { reason }` | The supervisor detected a non-consecutive sequence or a server-emitted replay-limit signal. Terminal for the same reason. |
| `MalformedEvent` | A CloudEvents id did not parse as `<event_type>@<u64>`. Terminal to avoid livelocking. |

The resilience layer absorbs transient errors internally (transport hiccups,
429/503, heartbeat timeouts) and reconnects with the right backoff. Those will
not surface as errors to your code.

## Building a custom state-store

Implement the `StateStore` trait:

```rust,ignore
use aviso::state::{Checkpoint, ResumeKey, StateStore, StoreError};

#[async_trait::async_trait]
impl StateStore for MyStore {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError> {
        // your read
    }
    async fn put(&self, key: &ResumeKey, cp: Checkpoint) -> Result<(), StoreError> {
        // your write. Must reject any cp whose last_committed_sequence <= the
        // currently-stored value, to keep at-least-once delivery sound.
    }
    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError> {
        // your delete
    }
}
```

The contract is linearisable: a successful `put` is committed-before-visible. A
failed `put` leaves all state unchanged. Strict monotonicity (no cursor moves
backwards) is mandatory.

## Where to go next

- [Rust API reference](../reference/rust-api.md): every type and method.
- [Architecture](./architecture.md): why the API is shaped this way.
- [Triggers overview](../triggers/overview.md): the trigger dispatcher contract.
