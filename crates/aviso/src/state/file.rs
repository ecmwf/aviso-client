//! File-backed implementation of [`StateStore`](super::StateStore).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::state::atomic_write::write_atomically;
use crate::state::resume_key::KEY_FORMAT_VERSION;
use crate::state::{Checkpoint, ResumeKey, StateStore, StoreError};

/// File-format version this client both reads and writes.
const FILE_FORMAT_VERSION: u32 = 1;

/// Length of a SHA-256 digest in bytes.
const DIGEST_BYTE_LEN: usize = 32;

/// Single-process file-backed state store.
///
/// # Runtime dependency
///
/// `JsonFileStore` uses [`tokio::task::spawn_blocking`] for all disk
/// I/O and must therefore be called from within a tokio runtime
/// context.
///
/// # Concurrency model
///
/// A dedicated disk-write mutex serialises writes. Each write builds
/// a candidate map outside the in-memory lock, performs the atomic
/// disk write outside the in-memory lock, and only mutates the
/// in-memory state after the disk write succeeds. Reads are never
/// blocked by an in-flight write beyond the brief installation of
/// the candidate.
///
/// Failed `put` and `delete` calls leave both disk and memory
/// unchanged.
///
/// # Linearizability scope: one open handle plus its clones
///
/// Linearizability holds across [`Clone`]s of a single
/// `JsonFileStore` handle. Two independent [`open`](Self::open)
/// calls to the same path return separate handles with independent
/// in-memory state and independent write mutexes; they DO NOT
/// coordinate, and concurrent writes against them can lose data
/// (the last writer's snapshot may not include the earlier
/// writer's commit). Always open the store once and share the
/// handle via `Clone`; never call `open` twice on the same path in
/// one process.
///
/// # Single-process only
///
/// This store does not perform cross-process locking either. Two
/// processes writing to the same file race; the loser's writes can
/// be lost. Cross-process safety is a follow-up on the roadmap.
///
/// # Async cancellation
///
/// `put` and `delete` are NOT cancellation-safe. If the future
/// returned by either method is dropped after the underlying
/// `spawn_blocking` has started the disk write but before the
/// in-memory install completes, the disk and in-memory state will
/// diverge: disk reflects the new value, memory reflects the old
/// one. Always drive `put`/`delete` to completion; do not race
/// them against `select!` arms that may cancel them.
///
/// # Not a polling store
///
/// External edits to the state file after [`open`](Self::open)
/// returns are NOT observed by an existing `JsonFileStore` handle.
/// Restart the process (or open a new handle) to pick up external
/// changes.
#[derive(Debug, Clone)]
pub struct JsonFileStore {
    path: Arc<PathBuf>,
    inner: Arc<RwLock<HashMap<ResumeKey, Checkpoint>>>,
    disk_write_mutex: Arc<Mutex<()>>,
}

/// On-disk JSON layout.
#[derive(Debug, Serialize, Deserialize)]
struct FileFormat {
    version: u32,
    key_format_version: u32,
    checkpoints: HashMap<String, Checkpoint>,
}

impl JsonFileStore {
    /// Open the state file at `path`, loading existing checkpoints
    /// into memory.
    ///
    /// The parent directory must exist; this constructor does not
    /// create it. The file itself is created lazily on first write.
    /// If the file is absent, the store starts empty.
    ///
    /// The file read happens on a tokio blocking thread.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Decode`] on malformed JSON,
    /// [`StoreError::UnsupportedFileVersion`] or
    /// [`StoreError::UnsupportedKeyFormatVersion`] on incompatible
    /// versions, [`StoreError::InvalidResumeKey`] on a non-hex or
    /// wrong-length resume key in the file, [`StoreError::Io`] on
    /// other I/O failures, or [`StoreError::BackgroundTaskFailed`]
    /// if the tokio blocking task that performs the file read
    /// itself fails (panic or cancellation).
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path: PathBuf = path.into();
        let read_path = path.clone();
        let initial = tokio::task::spawn_blocking(move || load_from_disk(&read_path))
            .await
            .map_err(StoreError::BackgroundTaskFailed)??;
        Ok(Self {
            path: Arc::new(path),
            inner: Arc::new(RwLock::new(initial)),
            disk_write_mutex: Arc::new(Mutex::new(())),
        })
    }

    async fn write_through<F>(&self, mutate: F) -> Result<(), StoreError>
    where
        F: FnOnce(&mut HashMap<ResumeKey, Checkpoint>) + Send,
    {
        let _disk_guard = self.disk_write_mutex.lock().await;

        let candidate = {
            let map = self.inner.read().await;
            let mut c = map.clone();
            mutate(&mut c);
            c
        };

        let file = FileFormat {
            version: FILE_FORMAT_VERSION,
            key_format_version: KEY_FORMAT_VERSION,
            checkpoints: candidate
                .iter()
                .map(|(k, v)| (k.as_hex(), v.clone()))
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(StoreError::Encode)?;

        let path = (*self.path).clone();
        tokio::task::spawn_blocking(move || write_atomically(&path, &bytes))
            .await
            .map_err(StoreError::BackgroundTaskFailed)??;

        let mut map = self.inner.write().await;
        *map = candidate;
        Ok(())
    }
}

#[async_trait]
impl StateStore for JsonFileStore {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError> {
        let map = self.inner.read().await;
        Ok(map.get(key).cloned())
    }

    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError> {
        let key = key.clone();
        self.write_through(move |map| {
            map.insert(key, checkpoint);
        })
        .await
    }

    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError> {
        let key = key.clone();
        self.write_through(move |map| {
            map.remove(&key);
        })
        .await
    }
}

fn load_from_disk(path: &Path) -> Result<HashMap<ResumeKey, Checkpoint>, StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(StoreError::Io(e)),
    };
    let file: FileFormat = serde_json::from_slice(&bytes).map_err(StoreError::Decode)?;
    if file.version > FILE_FORMAT_VERSION {
        return Err(StoreError::UnsupportedFileVersion {
            found: file.version,
            supported: FILE_FORMAT_VERSION,
        });
    }
    if file.key_format_version != KEY_FORMAT_VERSION {
        return Err(StoreError::UnsupportedKeyFormatVersion {
            found: file.key_format_version,
            supported: KEY_FORMAT_VERSION,
        });
    }
    let mut map = HashMap::with_capacity(file.checkpoints.len());
    for (hex_key, cp) in file.checkpoints {
        let raw = hex::decode(&hex_key).map_err(|e| StoreError::InvalidResumeKey {
            message: format!("'{hex_key}' is not valid hex: {e}"),
        })?;
        let digest: [u8; DIGEST_BYTE_LEN] =
            raw.try_into()
                .map_err(|v: Vec<u8>| StoreError::InvalidResumeKey {
                    message: format!(
                        "expected {DIGEST_BYTE_LEN}-byte digest, got {} bytes",
                        v.len()
                    ),
                })?;
        map.insert(ResumeKey::from_parts(digest, file.key_format_version), cp);
    }
    Ok(map)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use static_assertions::assert_impl_all;
    use tempfile::tempdir;
    use tokio::task::JoinSet;
    use url::Url;

    use super::{FILE_FORMAT_VERSION, JsonFileStore};
    use crate::state::resume_key::KEY_FORMAT_VERSION;
    use crate::state::{Checkpoint, ResumeKey, StateStore, StoreError};

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
    async fn corrupt_file_returns_decode_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{not valid json").unwrap();
        let result = JsonFileStore::open(&path).await;
        assert!(matches!(result, Err(StoreError::Decode(_))));
    }

    #[tokio::test]
    async fn newer_file_version_returns_unsupported_file_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let future = format!(
            r#"{{"version":{},"key_format_version":{},"checkpoints":{{}}}}"#,
            FILE_FORMAT_VERSION + 1,
            KEY_FORMAT_VERSION,
        );
        std::fs::write(&path, future).unwrap();
        let result = JsonFileStore::open(&path).await;
        match result {
            Err(StoreError::UnsupportedFileVersion { found, supported }) => {
                assert_eq!(found, FILE_FORMAT_VERSION + 1);
                assert_eq!(supported, FILE_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFileVersion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wrong_key_format_returns_unsupported_key_format() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mismatched = format!(
            r#"{{"version":{},"key_format_version":{},"checkpoints":{{}}}}"#,
            FILE_FORMAT_VERSION,
            KEY_FORMAT_VERSION + 1,
        );
        std::fs::write(&path, mismatched).unwrap();
        let result = JsonFileStore::open(&path).await;
        assert!(matches!(
            result,
            Err(StoreError::UnsupportedKeyFormatVersion { .. })
        ));
    }

    #[tokio::test]
    async fn invalid_hex_in_resume_key_returns_invalid_resume_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let bad = format!(
            r#"{{"version":{FILE_FORMAT_VERSION},"key_format_version":{KEY_FORMAT_VERSION},"checkpoints":{{"zzz":{{"last_committed_sequence":1,"last_event_id":null}}}}}}"#,
        );
        std::fs::write(&path, bad).unwrap();
        let result = JsonFileStore::open(&path).await;
        assert!(matches!(result, Err(StoreError::InvalidResumeKey { .. })));
    }

    #[tokio::test]
    async fn wrong_digest_length_returns_invalid_resume_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let bad = format!(
            r#"{{"version":{FILE_FORMAT_VERSION},"key_format_version":{KEY_FORMAT_VERSION},"checkpoints":{{"deadbeef":{{"last_committed_sequence":1,"last_event_id":null}}}}}}"#,
        );
        std::fs::write(&path, bad).unwrap();
        let result = JsonFileStore::open(&path).await;
        assert!(matches!(result, Err(StoreError::InvalidResumeKey { .. })));
    }

    #[tokio::test]
    async fn failed_write_leaves_in_memory_state_unchanged() {
        // Parent directory does not exist; atomic_write will fail on
        // temp-file creation. open() succeeds (file is treated as
        // missing). put() must fail AND must not poison in-memory.
        let nonexistent: PathBuf = "/nonexistent/aviso-state-test/state.json".into();
        let store = JsonFileStore::open(&nonexistent).await.unwrap();
        let result = store.put(&key(0), Checkpoint::new(1, None)).await;
        assert!(result.is_err(), "put on bad path must fail");
        assert!(
            store.get(&key(0)).await.unwrap().is_none(),
            "in-memory state must remain unchanged after a failed put"
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
        // Regression fixture for the type-doc warning under
        // "Linearizability scope". Two calls to `open` on the same
        // path produce independent in-memory state. Consumers must
        // open once and `Clone`; opening twice can lose writes.
        // Documented behaviour, not a bug. If we ever add a
        // process-local registry to coordinate independent opens,
        // this test inverts: each pair of opens shares state.
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
}
