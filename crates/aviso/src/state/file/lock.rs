// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Sidecar lockfile path: append `.lock` to the state file's name.
///
/// Sibling placement ensures same-filesystem locking semantics. The
/// fallback for paths without a file name (e.g., bare `/`) produces
/// `<parent>/.lock`, which is syntactically valid but semantically
/// indicates a misconfigured state path; the subsequent open call
/// surfaces the error.
pub(super) fn lock_path_for(state_path: &Path) -> PathBuf {
    let mut lock_name = state_path
        .file_name()
        .map(OsStr::to_os_string)
        .unwrap_or_default();
    lock_name.push(".lock");
    state_path.with_file_name(lock_name)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::lock_path_for;

    #[test]
    fn lock_path_for_appends_lock_suffix() {
        assert_eq!(
            lock_path_for(Path::new("/a/b/state.json")),
            PathBuf::from("/a/b/state.json.lock"),
        );
    }

    #[test]
    fn lock_path_for_handles_paths_without_extension() {
        assert_eq!(
            lock_path_for(Path::new("/a/b/state")),
            PathBuf::from("/a/b/state.lock"),
        );
    }

    #[test]
    fn lock_path_for_handles_bare_filename() {
        assert_eq!(
            lock_path_for(Path::new("state.json")),
            PathBuf::from("state.json.lock"),
        );
    }
}
