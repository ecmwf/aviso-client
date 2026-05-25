//! Shared tokio runtime used by every sync and async `PyO3` method.
//!
//! `pyo3-async-runtimes` manages a single multi-thread runtime keyed to
//! the Python process. Sync methods call [`runtime`] then `block_on` the
//! future inside `py.detach` to release the GIL during the await; async
//! methods use `pyo3_async_runtimes::tokio::future_into_py` from the same
//! crate, which schedules on the same runtime.

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    pyo3_async_runtimes::tokio::get_runtime()
}
