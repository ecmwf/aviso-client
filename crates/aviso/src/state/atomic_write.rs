//! Crash-safe atomic file write.
//!
//! Thin wrapper around the [`atomicwrites`] crate. Isolating it in
//! one private module means the upstream dependency surface can be
//! swapped or vendored without touching the file store's logic.
//!
//! The atomic write pattern: write to a temp file in the same
//! directory, `fsync` the temp file, atomically `rename` over the
//! target, then `fsync` the parent directory. On Windows the rename
//! step uses `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` and
//! `MOVEFILE_REPLACE_EXISTING`.
//!
//! These calls are synchronous (blocking). Async callers must wrap
//! them in [`tokio::task::spawn_blocking`].

use std::io;
use std::io::Write;
use std::path::Path;

use atomicwrites::{AllowOverwrite, AtomicFile};

/// Write `bytes` to `path` atomically and crash-safely.
///
/// # Errors
///
/// Returns the underlying [`io::Error`] from any I/O step (temp-file
/// creation, write, fsync, or rename).
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    AtomicFile::new(path, AllowOverwrite)
        .write(|f| f.write_all(bytes))
        .map_err(|e| match e {
            atomicwrites::Error::Internal(io_err) | atomicwrites::Error::User(io_err) => io_err,
        })
}
