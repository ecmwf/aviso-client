// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
