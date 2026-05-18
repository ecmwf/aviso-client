//! Error type returned by [`StateStore`](super::StateStore) operations.

use thiserror::Error;

/// Errors a [`StateStore`](super::StateStore) implementation can return.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum StoreError {
    /// I/O failure on the underlying state file.
    #[error("I/O error on state file: {0}")]
    Io(#[from] std::io::Error),

    /// In-memory state could not be serialised to JSON before writing.
    #[error("could not serialise state to JSON: {0}")]
    Encode(#[source] serde_json::Error),

    /// On-disk state file could not be parsed as JSON.
    #[error("could not parse state file: {0}")]
    Decode(#[source] serde_json::Error),

    /// File format version found on disk does not match the version
    /// this client reads and writes. Without a migration path, both
    /// newer (the client cannot interpret) and older (the file may
    /// have a different layout the current code does not understand)
    /// are rejected loudly rather than risk silent misinterpretation.
    #[error("state file format version {found} does not match supported version {supported}")]
    UnsupportedFileVersion {
        /// Version field read from the file.
        found: u32,
        /// Version this client reads and writes.
        supported: u32,
    },

    /// Key format version stored in the file does not match this client.
    ///
    /// Existing checkpoints become unreachable when this happens because
    /// keys cannot be re-derived from in-process state alone. The user
    /// must delete the file or upgrade the client.
    #[error("state file uses key_format_version {found}; this client uses {supported}")]
    UnsupportedKeyFormatVersion {
        /// Key format version read from the file.
        found: u32,
        /// Key format version this client uses.
        supported: u32,
    },

    /// A resume key entry in the file is not a valid hex-encoded SHA-256 digest.
    #[error("invalid resume key in state file: {message}")]
    InvalidResumeKey {
        /// Human-readable explanation (which key, why it failed).
        message: String,
    },

    /// The tokio blocking task that performed disk I/O failed.
    #[error("background task that performed the disk operation failed: {0}")]
    BackgroundTaskFailed(#[source] tokio::task::JoinError),
}
