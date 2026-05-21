//! Shared helpers for the CLI integration test suite.
//!
//! Every test in `crates/aviso-cli/tests/*.rs` invokes the `aviso`
//! binary via `assert_cmd::Command::cargo_bin("aviso")`. The
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
/// stripped so the test sees only what we set, and `AVISO_LOG` is
/// pinned to `error` so the test output is not polluted with INFO
/// startup events.
pub fn aviso() -> Command {
    let mut cmd = Command::cargo_bin("aviso").expect("aviso binary built");
    for (k, _) in std::env::vars() {
        if k.starts_with("AVISO_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("AVISO_LOG", "error");
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
