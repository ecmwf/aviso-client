# Resume & state

> Phase 0: placeholder. The full resume/checkpoint design lands with Phase 2 (state machine + `MemoryStore`) and Phase 3 (`JsonFileStore` with file lock).

## Operating contract (preview)

- Delivery is **at-least-once**.
- The client persists `last_committed_sequence` per **resume key** (a hash of server base URL + event type + canonical filter + schema fingerprint).
- On reconnect, the client re-issues the watch with `from_id = last_committed_sequence + 1`.
- A successful resume from stored state emits one `INFO` log with `event.name = "client.resume.applied"`.
- An explicit `replay-control: notification_replay_limit_reached` from the server is surfaced as a typed `HistoryGap` error and does **not** silently degrade to live-only.

See [ADR D2, D3, D15](../internals/decisions.md#d2--reconnect-as-norm--at-least-once--checkpointing) for the full design.
