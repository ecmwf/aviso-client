//! File-backed implementation of [`StateStore`](super::StateStore).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::state::atomic_write::write_atomically;
use crate::state::resume_key::KEY_FORMAT_VERSION;
use crate::state::{Checkpoint, ResumeKey, StateStore, StoreError};

mod load;
mod lock;
mod merge;

use load::load_from_disk;
use lock::lock_path_for;
use merge::merge_monotonic;

/// File-format version this client both reads and writes.
const FILE_FORMAT_VERSION: u32 = 1;

/// Length of a SHA-256 digest in bytes.
const DIGEST_BYTE_LEN: usize = 32;

/// File-backed state store, multi-process safe for cooperating
/// processes on local filesystems.
///
/// # Runtime dependency
///
/// `JsonFileStore` uses [`tokio::task::spawn_blocking`] for all disk
/// I/O and must therefore be called from within a tokio runtime
/// context.
///
/// # Concurrency model
///
/// A dedicated intra-process mutex serialises writes within a single
/// process. A cross-process `flock` on a sibling lockfile at
/// `<path>.lock` serialises writes between processes. Each write
/// clones the current map into a candidate under a brief shared
/// (read) lock on the in-memory state (which does not block other
/// readers), applies the mutation to the candidate, then runs
/// inside [`tokio::task::spawn_blocking`]: acquire the cross-process
/// exclusive lock, re-read the on-disk state, run a monotonic-cursor
/// merge against the candidate, perform the atomic disk write, and
/// release the lock. On success, briefly take the exclusive write
/// lock on the in-memory state to install the merged candidate.
/// Reads only block on writers during that final install step.
///
/// The merge step ensures that no checkpoint goes backwards under
/// concurrent writers: if a sibling process wrote a higher
/// `last_committed_sequence` for a key after this handle's last
/// observation, the merge keeps the sibling's value rather than
/// our stale candidate's value. Unrelated keys present on disk but
/// unknown to this handle are preserved. Intentional deletes are
/// honoured unless the on-disk sequence has advanced past the
/// value the handle observed at delete time (a concurrent writer
/// then wins).
///
/// Failed `put` and `delete` calls leave both disk and memory
/// unchanged.
///
/// # Concurrency scope
///
/// Writes are serialised and monotonic across both `Clone`s of a
/// single `JsonFileStore` handle and across separate processes that
/// open the same path: the intra-process mutex plus the
/// cross-process `flock` plus the strict-monotonic merge guarantee
/// that no commit is silently lost. A successful `put` from any
/// participant ends up visible to a subsequent re-open or to the
/// next write-through cycle of every other live handle.
///
/// Reads ([`get`](Self::get)) return this handle's in-memory
/// snapshot. Within a single handle (and its `Clone`s) the snapshot
/// is fully consistent: every successful `put` here is immediately
/// visible to a `get` here. Across independent handles (different
/// `open` calls in the same process, or other processes), the
/// snapshot can be stale until this handle's next write-through
/// cycle, which re-reads disk under the lock and merges in any
/// sibling commits. Consumers that need cross-handle read freshness
/// should either re-open the store or trigger a no-op write to
/// force a re-read.
///
/// Within a single process, prefer one [`open`](Self::open) +
/// [`Clone`] over multiple independent opens: it avoids the
/// per-handle in-memory drift and the redundant intra-process
/// mutexes.
///
/// # Lockfile precondition
///
/// The sidecar lockfile at `<path>.lock` must not be deleted,
/// renamed, or replaced while any `JsonFileStore` is using the
/// state file. The `flock` is attached to the inode; deleting and
/// recreating the path lets two writers acquire "the" lock on two
/// different inodes simultaneously, breaking mutual exclusion. The
/// library never deletes the lockfile; operators must not either.
///
/// # Local filesystems only
///
/// `flock(2)` is not safe over NFS, CIFS, or other network
/// filesystems. The state file and its lockfile must live on a
/// local filesystem.
///
/// # Async cancellation
///
/// `put` and `delete` are NOT cancellation-safe. If the future
/// returned by either method is dropped after the underlying
/// `spawn_blocking` has started but before the in-memory install
/// completes, the disk and in-memory state will diverge: disk
/// reflects the new value, memory reflects the old one. This handle
/// then returns stale data on subsequent [`get`](Self::get) calls
/// until it is dropped and re-opened.
///
/// The lock acquisition inside the spawned task is blocking and
/// can wait indefinitely if a sibling process holds the lock. A
/// dropped calling future does not cancel the spawned task, so the
/// disk-write window between cancellation and observable completion
/// can be wider than under the single-process design. Always drive
/// `put`/`delete` to completion; do not race them against `select!`
/// arms that may cancel them.
///
/// # Not a polling store
///
/// External edits to the state file by other processes are observed
/// only at the next `put` or `delete` (via the merge step) or after
/// a fresh [`open`](Self::open). A `get` that does not race a write
/// returns this handle's in-memory snapshot, which can be stale
/// compared to disk if siblings have written since this handle's
/// last write.
#[derive(Debug, Clone)]
pub struct JsonFileStore {
    path: Arc<PathBuf>,
    lock_path: Arc<PathBuf>,
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
        let lock_path = lock_path_for(&path);
        let read_path = path.clone();
        let initial = tokio::task::spawn_blocking(move || load_from_disk(&read_path))
            .await
            .map_err(StoreError::BackgroundTaskFailed)??;
        Ok(Self {
            path: Arc::new(path),
            lock_path: Arc::new(lock_path),
            inner: Arc::new(RwLock::new(initial)),
            disk_write_mutex: Arc::new(Mutex::new(())),
        })
    }

    async fn write_through<F>(&self, mutate: F) -> Result<(), StoreError>
    where
        F: FnOnce(&mut HashMap<ResumeKey, Checkpoint>) + Send,
    {
        let _disk_guard = self.disk_write_mutex.lock().await;

        let (pre_state, mut candidate) = {
            let map = self.inner.read().await;
            let pre = map.clone();
            let mut c = map.clone();
            mutate(&mut c);
            (pre, c)
        };

        // Enforce strict-monotonic put against pre_state, BEFORE the
        // merge sees disk. The merge's rule 1 only fires when disk
        // has a value >= candidate's; if disk is missing or older
        // than pre_state (external removal, rollback, or NFS-style
        // stale read), the merge would silently accept a regressing
        // candidate. Restore candidate[k] = pre_state[k] for every
        // key the mutator left at a lower or equal sequence to
        // pre_state. Intentional deletes (key absent from candidate)
        // skip this loop and reach the deletes map below.
        for (k, pre_cp) in &pre_state {
            if let Some(cand_cp) = candidate.get(k) {
                if cand_cp.last_committed_sequence <= pre_cp.last_committed_sequence {
                    candidate.insert(k.clone(), pre_cp.clone());
                }
            }
        }

        // Keys present in pre-state but absent from the candidate
        // are intentional deletes. Record each one's pre-state
        // sequence so the merge step can suppress the delete when a
        // concurrent writer has advanced the key past what this
        // handle observed.
        let deletes: HashMap<ResumeKey, u64> = pre_state
            .iter()
            .filter(|(k, _)| !candidate.contains_key(*k))
            .map(|(k, cp)| (k.clone(), cp.last_committed_sequence))
            .collect();

        let path = (*self.path).clone();
        let lock_path = (*self.lock_path).clone();

        let merged = tokio::task::spawn_blocking(move || -> Result<_, StoreError> {
            let lock_file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(StoreError::Io)?;
            let mut rw_lock = fd_lock::RwLock::new(lock_file);

            // EINTR retry: rustix's flock surfaces Interrupted on
            // signal delivery (SIGCHLD from spawned children,
            // SIGALRM from timers). Retry until acquired or a
            // non-Interrupted error surfaces. The empty Interrupted
            // arm yields () so the loop iterates without an
            // explicit `continue` (clippy::needless_continue).
            let _guard = loop {
                match rw_lock.write() {
                    Ok(g) => break g,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(StoreError::Io(e)),
                }
            };

            let disk_state = load_from_disk(&path)?;
            let disk_snapshot = disk_state.clone();
            let merged = merge_monotonic(candidate, disk_state, &deletes);

            // Skip the JSON encode + atomic write + fsync when the
            // resolved state matches what is already on disk: a
            // monotonically suppressed put (candidate seq <= disk
            // seq) or a delete of an absent key resolves to disk's
            // existing state, and rewriting it would add fsync
            // latency and lock hold time without changing anything.
            // Either way, return `merged` so the in-memory install
            // refreshes this handle's snapshot with any sibling
            // commits the lock acquisition exposed.
            if merged != disk_snapshot {
                let file = FileFormat {
                    version: FILE_FORMAT_VERSION,
                    key_format_version: KEY_FORMAT_VERSION,
                    checkpoints: merged
                        .iter()
                        .map(|(k, v)| (k.as_hex(), v.clone()))
                        .collect(),
                };
                let bytes = serde_json::to_vec_pretty(&file).map_err(StoreError::Encode)?;
                write_atomically(&path, &bytes)?;
            }

            Ok(merged)
        })
        .await
        .map_err(StoreError::BackgroundTaskFailed)??;

        let mut map = self.inner.write().await;
        *map = merged;
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

#[cfg(test)]
mod tests;
