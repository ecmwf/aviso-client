use std::collections::HashMap;
use std::path::Path;

use crate::state::resume_key::KEY_FORMAT_VERSION;
use crate::state::{Checkpoint, ResumeKey, StoreError};

use super::{DIGEST_BYTE_LEN, FILE_FORMAT_VERSION, FileFormat};

pub(super) fn load_from_disk(path: &Path) -> Result<HashMap<ResumeKey, Checkpoint>, StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File absent. Validate the parent so a misconfigured path
            // surfaces here rather than as an opaque atomic-write
            // failure on the first `put`. Distinguish missing parent
            // from a parent that exists but is not a directory; the
            // two are different operator errors.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    if !parent.exists() {
                        return Err(StoreError::Io(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("parent directory does not exist: {}", parent.display()),
                        )));
                    }
                    if !parent.is_dir() {
                        return Err(StoreError::Io(std::io::Error::new(
                            std::io::ErrorKind::NotADirectory,
                            format!("parent path is not a directory: {}", parent.display()),
                        )));
                    }
                }
            }
            return Ok(HashMap::new());
        }
        Err(e) => return Err(StoreError::Io(e)),
    };
    let file: FileFormat = serde_json::from_slice(&bytes).map_err(StoreError::Decode)?;
    if file.version != FILE_FORMAT_VERSION {
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
    use tempfile::tempdir;

    use super::load_from_disk;
    use crate::state::StoreError;

    use super::super::FILE_FORMAT_VERSION;
    use crate::state::resume_key::KEY_FORMAT_VERSION;

    #[tokio::test]
    async fn corrupt_file_returns_decode_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{not valid json").unwrap();
        let result = load_from_disk(&path);
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
        let result = load_from_disk(&path);
        match result {
            Err(StoreError::UnsupportedFileVersion { found, supported }) => {
                assert_eq!(found, FILE_FORMAT_VERSION + 1);
                assert_eq!(supported, FILE_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFileVersion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn older_file_version_also_returns_unsupported_file_version() {
        // A file claiming version 0 would have an unknown layout (we
        // have never written that version). Without a migration path,
        // silently accepting it risks misinterpreting the contents.
        // The gate rejects on `!=` rather than `>`, so older versions
        // surface as `UnsupportedFileVersion` too.
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");
        let older = r#"{"version":0,"key_format_version":1,"checkpoints":{}}"#;
        std::fs::write(&path, older).unwrap();
        let result = load_from_disk(&path);
        match result {
            Err(StoreError::UnsupportedFileVersion { found, supported }) => {
                assert_eq!(found, 0);
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
        let result = load_from_disk(&path);
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
        let result = load_from_disk(&path);
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
        let result = load_from_disk(&path);
        assert!(matches!(result, Err(StoreError::InvalidResumeKey { .. })));
    }

    #[tokio::test]
    async fn open_with_missing_parent_directory_errors() {
        // Construct a guaranteed-missing parent under a real tempdir
        // so the test is portable (no hard-coded absolute paths) and
        // deterministic (no risk of the path existing on someone's
        // machine). The "missing" subdirectory is never created;
        // `open` must see it as absent.
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing").join("state.json");
        let result = load_from_disk(&path);
        match result {
            Err(StoreError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected Io NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn open_with_parent_that_is_a_file_errors_distinctly() {
        // Parent path exists but is a regular file, not a directory.
        // Distinct from the missing-directory case: a different
        // operator mistake deserves a different ErrorKind so callers
        // can surface a different message.
        let dir = tempdir().unwrap();
        let regular_file = dir.path().join("not_a_dir");
        std::fs::write(&regular_file, b"this is a file, not a dir").unwrap();
        let state_path = regular_file.join("state.json");
        let result = load_from_disk(&state_path);
        match result {
            Err(StoreError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotADirectory);
            }
            other => panic!("expected Io NotADirectory, got {other:?}"),
        }
    }
}
