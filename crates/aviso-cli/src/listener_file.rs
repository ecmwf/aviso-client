// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Positional listener YAML files for `aviso listen` and
//! `aviso replay` per Amendment C.
//!
//! Each file carries its own top-level `listeners:` list (same
//! shape as the global config's `listeners:` block). The resolution
//! algorithm at the call site is:
//!
//! - With positional files: read each in argv order, concatenate
//!   the `listeners:` lists, return the result. Positional files
//!   REPLACE the global config's `listeners:` for this invocation.
//! - Without positional files: fall back to the global config's
//!   `listeners:`.
//! - Both empty: usage error (exit 2) with a helpful stderr
//!   message naming both paths checked.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_norway as yaml;

use crate::config::ListenerSpec;
use crate::paths;

/// Top-level shape of a positional listener YAML file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListenerFile {
    /// Listener entries. Each entry mirrors the global config's
    /// `listeners[i]` shape and reuses the same [`ListenerSpec`]
    /// type so trigger configs flow through `TriggerConfig` unchanged.
    #[serde(default)]
    pub(crate) listeners: Vec<ListenerSpec>,
}

/// Reads each `path` as a `ListenerFile` and returns the concatenated
/// `listeners:` lists in argv order.
///
/// # Errors
///
/// I/O failure on any file (file not readable, permission denied,
/// etc.), or YAML parse failure (with file:line:col surfaced via
/// the serde_norway Display impl).
pub(crate) fn load_concatenated(
    listener_paths: &[std::path::PathBuf],
) -> Result<Vec<ListenerSpec>> {
    let mut out = Vec::new();
    for path in listener_paths {
        let absolute =
            paths::absolutize(path).with_context(|| format!("at: {}", path.display()))?;
        let listeners =
            load_one(&absolute).with_context(|| format!("at: {}", absolute.display()))?;
        out.extend(listeners);
    }
    Ok(out)
}

fn load_one(path: &Path) -> Result<Vec<ListenerSpec>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read listener file: {}", path.display()))?;
    let file: ListenerFile = yaml::from_slice(&bytes)
        .with_context(|| format!("parse listener file: {}", path.display()))?;
    Ok(file.listeners)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on yaml round-trip is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn parse(yaml_text: &str) -> ListenerFile {
        yaml::from_str(yaml_text).expect("test YAML should parse")
    }

    #[test]
    fn empty_yaml_yields_empty_listeners() {
        let file = parse("listeners: []\n");
        assert!(file.listeners.is_empty());
    }

    #[test]
    fn single_listener_with_identifiers() {
        let file = parse("listeners:\n  - event: mars\n    identifiers:\n      class: od\n");
        assert_eq!(file.listeners.len(), 1);
        assert_eq!(file.listeners[0].event, "mars");
        assert_eq!(file.listeners[0].identifiers.len(), 1);
    }

    #[test]
    fn listener_preserves_structured_spatial_identifier() {
        let file = parse(
            "listeners:\n  - event: observations\n    identifiers:\n      point_cloud:\n        - [46, 8]\n        - [47, 9]\n",
        );
        assert_eq!(
            file.listeners[0].identifiers.get("point_cloud"),
            Some(&serde_json::json!([[46, 8], [47, 9]]))
        );
    }

    #[test]
    fn unknown_top_level_field_rejected() {
        let err = yaml::from_str::<ListenerFile>("bogus: 1\n").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("bogus") || msg.contains("unknown field"),
            "{msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn relative_listener_path_in_error_is_rendered_absolute() {
        let rel = std::path::PathBuf::from("aviso-test-nonexistent-listener-xyz.yaml");
        let err = load_concatenated(std::slice::from_ref(&rel)).unwrap_err();
        let chain = err
            .chain()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            chain.contains("/aviso-test-nonexistent-listener-xyz.yaml"),
            "error chain should contain absolute path (cwd-joined), got: {chain}"
        );
    }
}
