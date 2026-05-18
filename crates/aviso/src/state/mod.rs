//! Persistence for watch resume state.
//!
//! [`StateStore`] is an async trait `AvisoClient` consumes to persist
//! and retrieve [`Checkpoint`]s keyed by [`ResumeKey`]. Implementations
//! are `Send + Sync` and serialise concurrent writes internally so the
//! in-memory and (where applicable) on-disk states stay consistent.
//!
//! A successful [`StateStore::put`] is committed-before-visible: a
//! subsequent [`StateStore::get`] returns the value just written. For
//! durable implementations (such as [`JsonFileStore`]) the same
//! guarantee is durable-before-visible: the disk write returns
//! success before the new value becomes visible to readers. A failed
//! `put` leaves all state unchanged.
//!
//! Two implementations are provided:
//!
//! - [`MemoryStore`]: in-process. State dies with the program. Good
//!   for tests and short-lived consumers.
//! - [`JsonFileStore`]: backed by a JSON file with crash-safe atomic
//!   writes via the `atomicwrites` crate. Single-process; multi-process
//!   correctness is a follow-up.
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
/// internally. See the [module docs](self) for the linearizable-
/// semantics contract.
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Return the checkpoint stored at `key`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] or another I/O-related variant if
    /// the underlying storage cannot be read.
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError>;

    /// Store `checkpoint` at `key`, overwriting any existing value.
    ///
    /// On success the new value is committed and visible. For durable
    /// implementations the value is also persisted before this
    /// method returns. On error neither the in-memory nor (where
    /// applicable) the on-disk state is mutated.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if persistence fails.
    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError>;

    /// Remove the checkpoint at `key`. No-op if absent.
    ///
    /// On success the removal is committed and visible. For durable
    /// implementations the removal is also persisted before this
    /// method returns. On error neither the in-memory nor (where
    /// applicable) the on-disk state is mutated.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if persistence fails.
    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError>;
}
