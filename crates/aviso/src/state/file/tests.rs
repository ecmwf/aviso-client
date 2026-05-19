#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]

use serde_json::json;
use static_assertions::assert_impl_all;
use tempfile::tempdir;
use tokio::task::JoinSet;
use url::Url;

use super::*;
use crate::state::{Checkpoint, ResumeKey, StateStore};

assert_impl_all!(JsonFileStore: Send, Sync, Clone);

fn key(n: u8) -> ResumeKey {
    ResumeKey::new(
        &Url::parse("https://a/").unwrap(),
        "mars",
        &json!({"n": n}),
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn open_missing_file_starts_empty_and_does_not_create() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = JsonFileStore::open(&path).await.unwrap();
    assert!(store.get(&key(0)).await.unwrap().is_none());
    assert!(!path.exists(), "file must not be created until first write");
}

#[tokio::test]
async fn put_persists_across_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    {
        let store = JsonFileStore::open(&path).await.unwrap();
        store
            .put(&key(0), Checkpoint::new(42, Some("e@42".into())))
            .await
            .unwrap();
    }
    let reopened = JsonFileStore::open(&path).await.unwrap();
    let got = reopened.get(&key(0)).await.unwrap().unwrap();
    assert_eq!(got.last_committed_sequence, 42);
    assert_eq!(got.last_event_id.as_deref(), Some("e@42"));
}

#[tokio::test]
async fn delete_persists_across_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    {
        let store = JsonFileStore::open(&path).await.unwrap();
        store.put(&key(0), Checkpoint::new(1, None)).await.unwrap();
        store.put(&key(1), Checkpoint::new(2, None)).await.unwrap();
        store.delete(&key(0)).await.unwrap();
    }
    let reopened = JsonFileStore::open(&path).await.unwrap();
    assert!(reopened.get(&key(0)).await.unwrap().is_none());
    assert!(reopened.get(&key(1)).await.unwrap().is_some());
}

#[tokio::test]
async fn delete_absent_key_after_open_is_ok_and_file_remains_valid() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = JsonFileStore::open(&path).await.unwrap();
    store.delete(&key(0)).await.unwrap();
    let reopened = JsonFileStore::open(&path).await.unwrap();
    assert!(reopened.get(&key(0)).await.unwrap().is_none());
}

#[tokio::test]
async fn failed_write_leaves_in_memory_state_unchanged() {
    // Open succeeds in a real temp directory; first put commits
    // to disk and memory. Yank the parent directory; the second
    // put fails because the atomic-write target no longer has a
    // directory to land in. The in-memory state must still
    // reflect the first (successful) put, not the second
    // (failed) one. This is the linearizability property.
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = JsonFileStore::open(&path).await.unwrap();
    store.put(&key(0), Checkpoint::new(0, None)).await.unwrap();
    drop(dir); // tempdir cleans itself, taking the parent dir.
    let result = store.put(&key(1), Checkpoint::new(1, None)).await;
    assert!(result.is_err(), "put after yanked parent dir must fail");
    assert_eq!(
        store
            .get(&key(0))
            .await
            .unwrap()
            .unwrap()
            .last_committed_sequence,
        0,
        "first put's in-memory state must survive"
    );
    assert!(
        store.get(&key(1)).await.unwrap().is_none(),
        "failed second put must not appear in memory"
    );
}

#[tokio::test]
async fn after_put_file_parses_cleanly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = JsonFileStore::open(&path).await.unwrap();
    store
        .put(&key(0), Checkpoint::new(99, Some("e@99".into())))
        .await
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["version"], FILE_FORMAT_VERSION);
    assert_eq!(parsed["key_format_version"], KEY_FORMAT_VERSION);
    assert!(parsed["checkpoints"].is_object());
}

#[tokio::test]
async fn cloned_handles_share_state() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let a = JsonFileStore::open(&path).await.unwrap();
    let b = a.clone();
    a.put(&key(0), Checkpoint::new(7, None)).await.unwrap();
    assert_eq!(
        b.get(&key(0))
            .await
            .unwrap()
            .unwrap()
            .last_committed_sequence,
        7
    );
}

#[tokio::test]
async fn two_independent_opens_to_same_path_do_not_share_state() {
    // Regression fixture for the "Concurrency scope" type-doc.
    // Two calls to `open` on the same path produce independent
    // in-memory snapshots: the cross-process lock + monotonic
    // merge still guarantees commits cannot be silently lost
    // across opens, but a `get` on one handle does not see a
    // `put` on the other until that other handle's next
    // write-through cycle (or a re-open). Documented behaviour,
    // not a bug. If we ever add a process-local registry to
    // share state across independent opens, this test inverts.
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");

    let a = JsonFileStore::open(&path).await.unwrap();
    let b = JsonFileStore::open(&path).await.unwrap();

    a.put(&key(0), Checkpoint::new(1, None)).await.unwrap();
    assert!(
        b.get(&key(0)).await.unwrap().is_none(),
        "independent handle does not observe other handle's write through memory"
    );
}

#[tokio::test]
async fn concurrent_puts_to_distinct_keys_all_land() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = JsonFileStore::open(&path).await.unwrap();
    let mut set = JoinSet::new();
    for i in 0..10u8 {
        let s = store.clone();
        let k = key(i);
        set.spawn(async move {
            s.put(&k, Checkpoint::new(u64::from(i), None))
                .await
                .unwrap();
        });
    }
    while let Some(joined) = set.join_next().await {
        joined.unwrap();
    }
    for i in 0..10u8 {
        assert_eq!(
            store
                .get(&key(i))
                .await
                .unwrap()
                .unwrap()
                .last_committed_sequence,
            u64::from(i)
        );
    }
    let reopened = JsonFileStore::open(&path).await.unwrap();
    for i in 0..10u8 {
        assert_eq!(
            reopened
                .get(&key(i))
                .await
                .unwrap()
                .unwrap()
                .last_committed_sequence,
            u64::from(i)
        );
    }
}

#[tokio::test]
async fn put_lower_seq_is_no_op_even_when_disk_is_externally_rolled_back() {
    // Regression fixture: write_through must enforce strict-
    // monotonic put against pre_state (this handle's cached
    // in-memory state), not just against disk. If disk is
    // externally removed or rolled back to a stale value, the
    // merge alone would accept a regressing candidate; the
    // in-memory monotonic-restore loop catches it first.
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = JsonFileStore::open(&path).await.unwrap();

    store
        .put(&key(0), Checkpoint::new(10, Some("e@10".into())))
        .await
        .unwrap();

    // Externally remove the state file while the handle stays
    // open. The handle's in-memory snapshot still holds seq=10.
    std::fs::remove_file(&path).unwrap();

    // A put with seq=5 must NOT regress the handle's
    // committed state, even though disk no longer has a
    // sequence to compare against.
    store
        .put(&key(0), Checkpoint::new(5, Some("e@5".into())))
        .await
        .unwrap();

    let got = store.get(&key(0)).await.unwrap().unwrap();
    assert_eq!(
        got.last_committed_sequence, 10,
        "put(seq=5) against externally-removed disk must not regress pre_state seq=10",
    );
    assert_eq!(
        got.last_event_id.as_deref(),
        Some("e@10"),
        "restored value keeps pre_state metadata, not the suppressed put's metadata",
    );
}
