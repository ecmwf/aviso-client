//! `PyO3` bindings for [`aviso`].
//!
//! The Python extension exports a single module `aviso._native`. The Python
//! wrapper in `python/aviso/__init__.py` re-exports a curated subset under
//! the `aviso` package namespace; users `import aviso`, never
//! `import aviso._native`.
//!
//! Public Python surface: `VERSION`, the synchronous `AvisoClient` and
//! asynchronous `AsyncAvisoClient` with their `notify` / `schema` / admin /
//! `listen` methods, the value types they return (`Notification`,
//! `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`), the
//! `NotificationIterator` and `AsyncNotificationIterator` returned by
//! `listen`, the `WatchRequest` builder, the `Trigger` builder with its six
//! kind constructors, the five auth providers (`Bearer`, `Basic`, `Env`,
//! `ConfigFile`, `Chain`), the two state stores (`MemoryStore`,
//! `JsonFileStore`), and the exception hierarchy rooted at `AvisoError`.
//! The `_provoke_error` test helper is registered but kept off the
//! documented surface (used only by the python test suite).
//!
//! The full plan lives in `plans/python-api.md`.

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_log::{Caching, Logger};

mod auth;
mod clients;
mod error;
mod paths;
mod runtime;
mod state_stores;
mod streams;
mod triggers;
mod values;
mod watch;

/// Version string of the underlying [`aviso`] core crate.
///
/// The binding crate and the core crate share a workspace version, so a
/// single re-export keeps the surface honest: there is one version,
/// sourced from `Cargo.toml`.
pub const VERSION: &str = aviso::VERSION;

static LOGGER_INSTALLED: OnceLock<()> = OnceLock::new();

/// `aviso._native` `PyO3` extension module entry point.
///
/// Maturin builds the crate as a `cdylib` and embeds this function as the
/// module init under the name `aviso._native`. The function installs the
/// tracing-to-logging bridge once per process and registers the public
/// names listed on the module.
#[pymodule]
fn _native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    install_logging_bridge(py)?;
    error::register_exceptions(py, m)?;
    error::register_provoke_error(m)?;
    values::register_value_types(m)?;
    auth::register_auth(m)?;
    state_stores::register_state_stores(m)?;
    triggers::register_triggers(m)?;
    watch::register_watch(m)?;
    streams::register_streams(m)?;
    clients::register_clients(m)?;
    m.add("VERSION", VERSION)?;
    Ok(())
}

/// Installs the `tracing -> log -> python logging` bridge so events emitted
/// by the core crate's `tracing::info!` / `warn!` / `error!` calls surface
/// in the host application's Python `logging` configuration.
///
/// Idempotent. A concurrent first-time race between two threads importing
/// the module is benign: both threads call `Logger::new` (cheap, valid
/// handles) and `install` (one succeeds, the other returns
/// `SetLoggerError`); only one `set(())` commits.
///
/// A `Logger::new` failure (rare; would imply a broken pyo3-log
/// installation) propagates as `PyRuntimeError` and aborts module init
/// loudly. `OnceLock::get_or_init` is deliberately not used because its
/// closure cannot return `Result`; a failure would commit `()` to the cell
/// and a subsequent `import aviso` would short-circuit without retrying.
fn install_logging_bridge(py: Python<'_>) -> PyResult<()> {
    if LOGGER_INSTALLED.get().is_some() {
        return Ok(());
    }
    let logger = Logger::new(py, Caching::Loggers)
        .map_err(|e| PyRuntimeError::new_err(format!("pyo3-log Logger::new failed: {e}")))?;
    // reason: SetLoggerError means another bridge is already installed
    // for this process (a host application has its own log facade). The
    // library does not fight an existing installation; treating that one
    // failure mode as success is the documented behaviour.
    logger.install().ok();
    // reason: benign race between concurrent first-time imports. Only one
    // thread's `set(())` commits; the other is a no-op because the cell
    // already holds `()`. Either order is correct because the work above
    // is idempotent.
    LOGGER_INSTALLED.set(()).ok();
    Ok(())
}
