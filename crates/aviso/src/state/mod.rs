// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Persistence for watch resume state.
//!
//! [`StateStore`] is an async trait for persisting and retrieving
//! [`Checkpoint`]s keyed by [`ResumeKey`], consumed by the aviso
//! watch supervisor for at-least-once delivery across reconnects and
//! process restarts. Implementations are `Send + Sync` and serialise
//! concurrent writes internally so the in-memory and (where
//! applicable) on-disk states stay consistent.
//!
//! A successful [`StateStore::put`] is committed-before-visible: a
//! subsequent [`StateStore::get`] returns the resolved value. For
//! durable implementations (such as [`JsonFileStore`]) the same
//! guarantee is durable-before-visible: the disk write returns
//! success before the new value becomes visible to readers. A failed
//! `put` leaves all state unchanged.
//!
//! `put` is **strictly monotonic** in `last_committed_sequence`: a
//! put with a sequence less than or equal to the existing value is a
//! silent no-op. Both implementations enforce this so consumers see
//! identical behaviour regardless of the backing store. Callers
//! needing to reset a checkpoint to a lower sequence must
//! [`delete`](StateStore::delete) first, then `put`.
//!
//! Two implementations are provided:
//!
//! - [`MemoryStore`]: in-process. State dies with the program. Good
//!   for tests and short-lived consumers.
//! - [`JsonFileStore`]: backed by a JSON file with crash-safe atomic
//!   writes via the `atomicwrites` crate plus a cross-process
//!   advisory lock on a sidecar lockfile. Safe for cooperating
//!   processes on local filesystems; see the type docs for the
//!   precondition list (no NFS/CIFS, no external deletion of the
//!   lockfile).
//!
//! Resume keys are derived from a base URL, an event type, the watch
//! filter body, and an optional schema fingerprint (D3 in the ADR log).
//! The hash deliberately excludes any server-side resume position; the
//! same logical subscription always computes the same key.

mod atomic_write;
mod checkpoint;
mod error;
mod file;
mod memory;
mod resume_key;

pub use checkpoint::Checkpoint;
pub use error::StoreError;
pub use file::JsonFileStore;
pub use memory::MemoryStore;
pub use resume_key::{ResumeKey, ResumeKeyError};

use async_trait::async_trait;

/// Persistent storage for [`Checkpoint`] keyed by [`ResumeKey`].
///
/// Implementations are `Send + Sync` and serialise concurrent writes
/// internally. See the [module docs](self) for the strict-monotonic
/// `put` contract and the multi-writer concurrency notes that
/// distinguish [`MemoryStore`] from [`JsonFileStore`].
///
/// # Cancel-safety expectation for watch supervisor consumers
///
/// The watch supervisor in [`crate::AvisoClient::watch`] consumes this
/// trait with an asymmetric cancel-safety contract:
///
/// - [`Self::get`] MAY be cancelled (the future MAY be dropped
///   mid-flight). The supervisor races the initial cursor-load `get`
///   against per-stream drop and parent drop via `tokio::select!`, so
///   a long-running `get` that overlaps a drop terminates promptly.
///   Implementations must be safe to drop mid-`await`: dropping the
///   future must not corrupt internal state, leak resources beyond
///   what `Drop` cleans up, or break invariants for the next `get` or
///   `put` against the same key. Pure I/O-bound implementations
///   (memory map, file read, network round trip) usually satisfy this
///   trivially; implementations that hold a partially-completed
///   internal transaction across the `await` boundary must release it
///   on drop.
///
/// - [`Self::put`] is NOT cancelled. The supervisor lets in-progress
///   puts run to completion so the underlying durable state is never
///   left half-written. Implementations may rely on atomic completion
///   within their own `await` lifetime. The trade-off is bounded extra
///   exit latency proportional to the `put` duration on parent drop.
///
/// Implementations SHOULD keep both methods bounded in wall-clock time
/// (the shipped [`JsonFileStore`] is dominated by `fsync`, typically
/// tens of milliseconds on local disk). A custom store that performs
/// an unbounded network call from within `put` will extend
/// `AvisoClient::Drop` latency by that amount; if that is
/// unacceptable, the implementation may internally apply its own
/// timeout and return [`StoreError`] on expiry. The supervisor will
/// surface that timeout as [`crate::ClientError::StateStore`] and
/// terminate the watch.
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Return the checkpoint stored at `key`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] or another I/O-related variant if
    /// the underlying storage cannot be read.
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError>;

    /// Store `checkpoint` at `key`.
    ///
    /// Implementations must be **strictly monotonic** in
    /// `last_committed_sequence`: a `put` whose sequence is less than
    /// or equal to the existing value's sequence is silently a no-op.
    /// `last_committed_sequence` is the load-bearing forward-only
    /// cursor the supervisor reconnects from; allowing it to move
    /// backwards would silently lose at-least-once delivery
    /// guarantees, both within a single process (where a stale
    /// callback could clobber a fresh advance) and across cooperating
    /// processes (where a fresh handle reading disk and writing a
    /// lower value would overwrite the durable state).
    ///
    /// Callers that need to reset a checkpoint to a lower sequence
    /// must [`delete`](Self::delete) the key first, then `put` the
    /// new value.
    ///
    /// On success (whether the write landed or was monotonically
    /// suppressed) the resolved value is committed and visible. For
    /// durable implementations the resolved value is also persisted
    /// before this method returns. On error neither the in-memory
    /// nor (where applicable) the on-disk state is mutated.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if persistence fails.
    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError>;

    /// Remove the checkpoint at `key`. No-op if absent.
    ///
    /// Multi-writer implementations (such as [`JsonFileStore`])
    /// may leave the key present on durable storage when a
    /// concurrent writer's value coexists with this caller's
    /// `delete`. Two distinct races trigger this:
    ///
    /// 1. **Caller observed the key**: the caller's pre-state held
    ///    `(key, seq=N)`. A sibling process advanced it to
    ///    `(key, seq=N+M)` between the pre-state load and the
    ///    write-through merge. The "delete the K I knew" intent
    ///    becomes ambiguous in that race; the implementation
    ///    preserves the sibling's later value.
    /// 2. **Caller observed the key absent**: the caller's
    ///    pre-state did not contain `key` at all. A sibling
    ///    process created it (any sequence) between the
    ///    pre-state load and the write-through merge. The
    ///    implementation cannot distinguish "delete a key I never
    ///    saw" from "did not touch this key" through the mutator
    ///    API, so the sibling's value is preserved.
    ///
    /// A subsequent [`get`](Self::get) then returns the concurrent
    /// value rather than `None`. Callers that need a strict
    /// remove-and-confirm semantic must re-read and retry
    /// (or accept the at-least-once delivery the supervisor uses,
    /// which treats either outcome as valid). Single-writer
    /// implementations (such as [`MemoryStore`]) always honour the
    /// delete because there is no concurrent-writer race to resolve.
    ///
    /// On success the resolved state (either the delete or the
    /// preserved concurrent value) is committed and visible. For
    /// durable implementations the resolved state is also persisted
    /// before this method returns. On error neither the in-memory
    /// nor (where applicable) the on-disk state is mutated.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if persistence fails.
    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError>;
}
