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

/// Returns the base URL the e2e stack runs at. Resolves `AVISO_E2E_BASE_URL` first, then
/// `AVISO_BASE_URL` (the variable the docs and the Python fixtures use), then falls back to
/// [`DEFAULT_BASE_URL`]. The e2e-specific variant wins on conflict so a developer can override
/// the local stack independently of the `AVISO_BASE_URL` they have set for other purposes.
#[must_use]
pub fn base_url() -> String {
    std::env::var("AVISO_E2E_BASE_URL")
        .or_else(|_| std::env::var("AVISO_BASE_URL"))
        .unwrap_or_else(|_| DEFAULT_BASE_URL.into())
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
