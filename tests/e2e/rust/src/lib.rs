//! Shared helpers for end-to-end Rust tests against the docker-compose stack at `tests/e2e/`.
//!
//! Every integration test under `tests/` is `#[ignore]`-gated. The CLI tests use
//! `assert_cmd::Command::cargo_bin("aviso")` which expects `target/debug/aviso` to exist, so
//! build the CLI first. Full local-run flow from the repo root:
//!
//! ```text
//! bash tests/e2e/shared/stack_up.sh
//! cargo build -p aviso-cli
//! cargo test --locked -p aviso-e2e -- --include-ignored --test-threads=1
//! ```

use std::process::Command;
use std::sync::Arc;

use aviso::AvisoClient;
use aviso::auth::Basic;

/// Default base URL for the local aviso-server (overridden by `AVISO_E2E_BASE_URL`).
pub const DEFAULT_BASE_URL: &str = "http://localhost:8000";

/// Plain-provider account with the `producer` role: can publish + read.
pub const PRODUCER_USERNAME: &str = "producer-user";
/// Producer-account password.
pub const PRODUCER_PASSWORD: &str = "producer-pass";

/// Plain-provider account with the `reader` role: can read but not publish.
pub const READER_USERNAME: &str = "reader-user";
/// Reader-account password.
pub const READER_PASSWORD: &str = "reader-pass";

/// Plain-provider account with the `admin` role: can call admin endpoints.
pub const ADMIN_USERNAME: &str = "admin-user";
/// Admin-account password.
pub const ADMIN_PASSWORD: &str = "admin-pass";

/// Returns the base URL the e2e stack runs at. Resolution order:
///
/// 1. `AVISO_E2E_BASE_URL` (e2e-specific override; wins so a developer can target a different
///    stack from any `AVISO_BASE_URL` they may have set for other purposes).
/// 2. `AVISO_BASE_URL` (the variable the docs and the Python fixtures use).
/// 3. `http://localhost:${AVISO_SERVER_HOST_PORT:-8000}` (matches `stack_up.sh`'s port
///    override; if the operator started the stack on a non-default port via that variable,
///    the Rust tests pick it up automatically).
#[must_use]
pub fn base_url() -> String {
    if let Ok(explicit) = std::env::var("AVISO_E2E_BASE_URL") {
        return explicit;
    }
    if let Ok(explicit) = std::env::var("AVISO_BASE_URL") {
        return explicit;
    }
    let port = std::env::var("AVISO_SERVER_HOST_PORT").unwrap_or_else(|_| "8000".into());
    format!("http://localhost:{port}")
}

/// Returns an `AvisoClient` authenticated as `producer-user`.
///
/// # Errors
///
/// Propagates `aviso::ClientError` if the client cannot be built.
pub fn producer_client() -> aviso::Result<AvisoClient> {
    AvisoClient::builder()
        .base_url(base_url())
        .auth(Arc::new(Basic::new(PRODUCER_USERNAME, PRODUCER_PASSWORD)?))
        .build()
}

/// Returns an `AvisoClient` authenticated as `reader-user`.
///
/// # Errors
///
/// Propagates `aviso::ClientError` if the client cannot be built.
pub fn reader_client() -> aviso::Result<AvisoClient> {
    AvisoClient::builder()
        .base_url(base_url())
        .auth(Arc::new(Basic::new(READER_USERNAME, READER_PASSWORD)?))
        .build()
}

/// Returns a `std::process::Command` pointing at the `aviso` binary with all `AVISO_*`
/// environment variables stripped (so the developer's interactive `AVISO_BASE_URL`,
/// `AVISO_TOKEN`, `AVISO_LOG`, and the like never leak into a test) and
/// `AVISO_CLIENT_CONFIG_FILE` pointed at a non-existent path so the CLI does not parse
/// the developer's real `~/.config/aviso/config.yaml`.
///
/// Mirrors the isolation the hermetic CLI test helper at
/// `crates/aviso-cli/tests/common/mod.rs::aviso` applies. Use this for every e2e CLI
/// invocation so local machine state cannot make a test pass or fail for the wrong
/// reason. Wrap in `assert_cmd::Command::from_std(...)` for the timeout +
/// `.assert().success()` pattern.
///
/// # Panics
///
/// Panics if the `aviso` binary is not built at `target/debug/aviso`; run
/// `cargo build -p aviso-cli` first.
#[must_use]
pub fn isolated_aviso_command() -> Command {
    let bin = assert_cmd::cargo::cargo_bin("aviso");
    let mut cmd = Command::new(bin);
    for (k, _) in std::env::vars() {
        if k.starts_with("AVISO_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("AVISO_LOG", "error");
    cmd.env(
        "AVISO_CLIENT_CONFIG_FILE",
        "/nonexistent/aviso-e2e-isolated/config.yaml",
    );
    cmd
}
