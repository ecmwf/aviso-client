// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Shared helpers for the CLI integration test suite.
//!
//! Every test in `crates/aviso-cli/tests/*.rs` invokes the `aviso`
//! binary via `assert_cmd` and its `cargo_bin!` macro. The
//! helpers in this module wrap the common patterns: building a
//! `Command` with deterministic env (no inherited `AVISO_*`
//! variables, no `AVISO_LOG` filter), pointing the CLI at a
//! per-test temp directory for config and state files, and
//! starting a wiremock server pre-configured for the most common
//! response shapes.

#![allow(
    dead_code,
    reason = "test helpers shared across integration test crates; not every helper is used in every file"
)]

use std::path::Path;

use assert_cmd::Command;

/// Builds a fresh `assert_cmd::Command` for the `aviso` binary
/// with deterministic environment: every `AVISO_*` env variable is
/// stripped so the test sees only what we set, `AVISO_LOG` is
/// pinned to `error` so the test output is not polluted with INFO
/// events, and `AVISO_CLIENT_CONFIG_FILE` is pointed at a
/// non-existent path so tests are isolated from the operator's real
/// `~/.config/aviso/config.yaml` on the test machine (otherwise a
/// local config file with `base_url:` or `auth:` would make several
/// tests non-deterministic by silently overriding the test fixture
/// with whichever values the developer set for interactive use).
///
/// `AVISO_STATE_FILE` is NOT overridden here because `aviso listen`
/// and `aviso replay` exercise state-store creation as part of their
/// happy path; tests that touch listen/replay opt into isolation
/// via `--no-state-store` or `--state-file <path>` explicitly.
pub fn aviso() -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("aviso"));
    for (k, _) in std::env::vars() {
        if k.starts_with("AVISO_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("AVISO_LOG", "error");
    cmd.env(
        "AVISO_CLIENT_CONFIG_FILE",
        "/nonexistent/aviso-test-isolated/config.yaml",
    );
    // Without this the credentials tier falls back to the real home
    // directory, so a developer with credentials on disk would get
    // different results from CI.
    cmd.env(
        "AVISO_CREDENTIALS_FILE",
        "/nonexistent/aviso-test-isolated/credentials.yaml",
    );
    cmd
}

/// Same as [`aviso`] but with the `--config` flag pointing at
/// `config_path` (which may or may not exist; absent files are
/// treated as empty configs).
pub fn aviso_with_config(config_path: &Path) -> Command {
    let mut cmd = aviso();
    cmd.arg("--config").arg(config_path);
    cmd
}
