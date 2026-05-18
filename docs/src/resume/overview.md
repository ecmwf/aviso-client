# Resume and state

`aviso-server` deliberately closes a watch connection after its `connection_max_duration_sec` (default 3600 s). Reconnects are routine, not failures. For the client to pick up where it left off, it needs to remember the last successfully processed sequence number per logical subscription, and to use it on reconnect via the `from_id` parameter in the POST body.

The `aviso::state` module ships the storage layer for that. Three concrete moving parts plus a trait.

## The contract: `StateStore`

```rust,ignore
use aviso::state::{Checkpoint, ResumeKey, StateStore, StoreError};

#[async_trait::async_trait]
pub trait StateStore: Send + Sync {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError>;
    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError>;
    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError>;
}
```

The trait promises **linearizable semantics**: a successful `put` is committed-before-visible. A subsequent `get` (from any clone of the same store) is guaranteed to return what was just written. For durable implementations (such as `JsonFileStore` below) the same guarantee is durable-before-visible: the disk write returns success before the new value becomes visible to readers. A failed `put` leaves all state unchanged.

Implementations serialise concurrent writes internally so the in-memory and (where applicable) on-disk views stay consistent.

## The key: `ResumeKey`

A logical identifier for a subscription. Two `AvisoClient` instances pointed at the same server, watching the same event type with the same filter, always compute the same key.

```rust,ignore
use aviso::state::ResumeKey;
use serde_json::json;
use url::Url;

let key = ResumeKey::new(
    &Url::parse("https://aviso.example.com/")?,
    "mars",
    &json!({ "class": "od", "stream": "oper" }),
    None, // optional schema fingerprint
)?;
```

The key is a SHA-256 digest plus a `key_format_version`. Inputs that go into the digest:

- Base URL (normalised: lowercase scheme and host, default ports stripped, userinfo removed).
- Event type.
- The filter body, canonicalised via RFC 8785 JSON Canonicalization Scheme. Object-key reordering does NOT change the key.
- Optional schema fingerprint.

The hash deliberately excludes any resume position. `from_id` and `from_date` are how the client tells the server where to start; they are not subscription identity (D3 in the ADR log).

## The value: `Checkpoint`

```rust,ignore
use aviso::state::Checkpoint;

let cp = Checkpoint::new(12345, Some("mars@12345".into()));
```

`last_committed_sequence` is the canonical resume cursor. The reconnect path sends `from_id = last_committed_sequence + 1`. `last_event_id` is the human-readable form of the same notification; it exists for `tracing` logs and operator debugging and is not consulted on reconnect.

## The implementations

### `MemoryStore`

```rust,ignore
use aviso::state::MemoryStore;

let store = MemoryStore::new();
```

In-process. State lives in an `Arc<RwLock<HashMap<...>>>`. `Clone` is cheap; cloned handles share the same map. State is lost when the last handle drops.

Use it for tests, short-lived CLI invocations, and any consumer that is fine starting over after a restart.

### `JsonFileStore`

```rust,ignore
use aviso::state::JsonFileStore;
use std::path::PathBuf;

// `JsonFileStore::open` takes a real filesystem path. Rust's
// `Path`/`PathBuf` do not expand `~`, so pick an absolute path or
// resolve the home directory yourself (the `directories` or
// `etcetera` crate does this on every supported OS).
let path = PathBuf::from("/var/lib/aviso/state.json");
let store = JsonFileStore::open(&path).await?;
```

Backed by a JSON file with crash-safe atomic writes. Each write goes to a temp file in the same directory, the temp file is `fsync`ed, then atomically renamed over the target, then the parent directory is `fsync`ed (on POSIX). On Windows the rename step uses `MoveFileExW` with `MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING`. A `kill -9` mid-write cannot corrupt the existing file.

The store is single-process AND single-handle: linearizability holds across `Clone`s of one handle, but two independent `open` calls to the same path do not coordinate (separate in-memory state, separate write mutexes; concurrent writes can lose data). Open once, share via `Clone`. Cross-process safety is a separate follow-up.

`put` and `delete` are NOT cancellation-safe; drive them to completion and do not race them against `select!` arms that may cancel.

It is NOT a polling store: external edits to the file after `open` returns are not observed. Restart the process (or open a new handle) to pick up external changes.

The parent directory must exist; `open` does not create it. The file itself is created lazily on first write.

## Choosing a path

The library exposes the trait so any consumer can choose any path or backend today. The aviso CLI binary will default to `~/.config/aviso/state.json` once it wires `StateStore` (a follow-up; the CLI does not yet consume the state module). When that wiring lands, the CLI is responsible for expanding the literal tilde itself before the path reaches `JsonFileStore::open`, since `Path`/`PathBuf` do not perform that expansion.

## Architectural references

- D3: resume key shape (`docs/src/internals/decisions.md`).
- D4: store trait, the impls, and the file format.
