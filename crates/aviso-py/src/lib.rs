// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `PyO3` bindings for [`aviso`].
//!
//! The Python extension exports a single module `pyaviso._native`. The Python
//! wrapper in `python/pyaviso/__init__.py` re-exports a curated subset under
//! the `pyaviso` package namespace; users `import pyaviso`, never
//! `import pyaviso._native`.
//!
//! Public Python surface: `VERSION`, the synchronous `AvisoClient` and
//! asynchronous `AsyncAvisoClient` with their `notify` / `schema` / admin /
//! `listen` methods, the value types they return (`Notification`,
//! `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`), the
//! `NotificationIterator` and `AsyncNotificationIterator` returned by
//! `listen`, the `WatchRequest` builder, the `Trigger` builder with its six
//! kind constructors, the five auth providers (`Bearer`, `Basic`, `Env`,
//! `ConfigFile`, `Chain`) and the `Anonymous` marker that turns credential
//! discovery off, the two state stores (`MemoryStore`,
//! `JsonFileStore`), and the exception hierarchy rooted at `AvisoError`.
//! The `_provoke_error` test helper is registered but kept off the
//! documented surface (used only by the python test suite).
//!
//! Architectural decisions live in `plans/decisions.md`.

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use log::{Log, Metadata, Record};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3_log::{Caching, Logger};

mod auth;
mod cli;
mod client_args;
mod clients;
mod config;
mod error;
mod error_test_helper;
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

/// Prevents Rust worker threads from entering Python after interpreter
/// shutdown has started.
///
/// `pyo3-log` uses `Python::attach` internally. That is appropriate while
/// Python is running, but a late Tokio or HTTP connection-cleanup log can
/// arrive while the interpreter is being finalized. Acquiring Python through
/// `try_attach` first keeps the interpreter attached for the whole forwarding
/// call and drops only those records which arrive too late to be delivered.
struct FinalizationSafeLogger<L> {
    inner: L,
}

impl<L> FinalizationSafeLogger<L> {
    fn new(inner: L) -> Self {
        Self { inner }
    }
}

impl<L: Log> Log for FinalizationSafeLogger<L> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        let _ = Python::try_attach(|_| self.inner.log(record));
    }

    fn flush(&self) {
        let _ = Python::try_attach(|_| self.inner.flush());
    }
}

/// `pyaviso._native` `PyO3` extension module entry point.
///
/// Maturin builds the crate as a `cdylib` and embeds this function as the
/// module init under the name `pyaviso._native`. The function installs the
/// tracing-to-logging bridge once per process and registers the public
/// names listed on the module.
#[pymodule]
fn _native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    install_logging_bridge(py)?;
    error::register_exceptions(py, m)?;
    error_test_helper::register_provoke_error(m)?;
    values::register_value_types(m)?;
    auth::register_auth(m)?;
    state_stores::register_state_stores(m)?;
    triggers::register_triggers(m)?;
    watch::register_watch(m)?;
    streams::register_streams(m)?;
    config::register(m)?;
    clients::register_clients(m)?;
    cli::register_cli(m)?;
    m.add("VERSION", VERSION)?;
    Ok(())
}

/// Installs the `tracing -> log -> python logging` bridge so events emitted
/// by the core crate's `tracing::info!` / `warn!` / `error!` calls surface
/// in the host application's Python `logging` configuration.
///
/// Idempotent. A concurrent first-time race between two threads importing
/// the module is benign: both threads construct a logger and try to install
/// it (one succeeds, the other receives `SetLoggerError`); only one `set(())`
/// commits.
///
/// A `Logger::new` failure (rare; would imply a broken pyo3-log
/// installation) propagates as `PyRuntimeError` and aborts module init
/// loudly. `OnceLock::get_or_init` is deliberately not used because its
/// closure cannot return `Result`; a failure would commit `()` to the cell
/// and a subsequent `import pyaviso` would short-circuit without retrying.
fn install_logging_bridge(py: Python<'_>) -> PyResult<()> {
    if LOGGER_INSTALLED.get().is_some() {
        return Ok(());
    }
    let logger = Logger::new(py, Caching::Loggers)
        .map_err(|e| PyRuntimeError::new_err(format!("pyo3-log Logger::new failed: {e}")))?;
    let logger = FinalizationSafeLogger::new(logger);
    // reason: SetLoggerError means another bridge is already installed
    // for this process (a host application has its own log facade). The
    // library does not fight an existing installation; treating that one
    // failure mode as success is the documented behaviour.
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
    // reason: benign race between concurrent first-time imports. Only one
    // thread's `set(())` commits; the other is a no-op because the cell
    // already holds `()`. Either order is correct because the work above
    // is idempotent.
    LOGGER_INSTALLED.set(()).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use log::Level;

    use super::*;

    struct CountingLogger {
        records: Arc<AtomicUsize>,
    }

    impl Log for CountingLogger {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn log(&self, _record: &Record<'_>) {
            self.records.fetch_add(1, Ordering::Relaxed);
        }

        fn flush(&self) {}
    }

    #[test]
    fn finalization_safe_logger_forwards_worker_thread_records() {
        Python::attach(|_| {});
        let records = Arc::new(AtomicUsize::new(0));
        let logger = FinalizationSafeLogger::new(CountingLogger {
            records: Arc::clone(&records),
        });

        let worker = std::thread::spawn(move || {
            let record = Record::builder()
                .args(format_args!("worker record"))
                .level(Level::Info)
                .target("aviso_py::test")
                .build();
            logger.log(&record);
        });

        assert!(worker.join().is_ok(), "worker logger thread panicked");
        assert_eq!(records.load(Ordering::Relaxed), 1);

        let metadata = Metadata::builder()
            .level(Level::Info)
            .target("aviso_py::test")
            .build();
        assert!(FinalizationSafeLogger::new(CountingLogger { records }).enabled(&metadata));
    }
}
