// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `PyO3` wrappers for `MemoryStore` and `JsonFileStore`.

#![allow(
    clippy::unused_self,
    reason = "Python's __repr__ protocol passes &self by convention; MemoryStore has no \
              fields to render so the method returns a constant marker string"
)]

use std::sync::Arc;

use aviso::state::{JsonFileStore, MemoryStore, StateStore};
use pyo3::prelude::*;

use crate::error::map_client_error;
use crate::paths::normalize_path;
use crate::runtime::runtime;

/// Thin Python wrapper around `aviso::state::MemoryStore`.
#[pyclass(name = "MemoryStore", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyMemoryStore {
    inner: Arc<dyn StateStore>,
}

#[pymethods]
impl PyMemoryStore {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(MemoryStore::default()),
        }
    }

    fn __repr__(&self) -> &'static str {
        "MemoryStore()"
    }
}

impl PyMemoryStore {
    pub(crate) fn store(&self) -> Arc<dyn StateStore> {
        Arc::clone(&self.inner)
    }
}

/// Thin Python wrapper around `aviso::state::JsonFileStore`.
#[pyclass(
    name = "JsonFileStore",
    module = "pyaviso._native",
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyJsonFileStore {
    inner: Arc<dyn StateStore>,
    path: String,
}

#[pymethods]
impl PyJsonFileStore {
    #[new]
    fn new(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        let path_buf = normalize_path(py, path)?;
        let store_path = path_buf.clone();
        let result = py.detach(|| runtime().block_on(JsonFileStore::open(&store_path)));
        let store = result.map_err(|e| map_client_error(py, aviso::ClientError::StateStore(e)))?;
        let path_display = path_buf.display().to_string();
        Ok(Self {
            inner: Arc::new(store),
            path: path_display,
        })
    }

    fn __repr__(&self) -> String {
        format!("JsonFileStore({:?})", self.path)
    }
}

impl PyJsonFileStore {
    pub(crate) fn store(&self) -> Arc<dyn StateStore> {
        Arc::clone(&self.inner)
    }
}

/// Extracts an `Arc<dyn StateStore>` from either wrapper.
pub(crate) fn extract_store(obj: &Bound<'_, PyAny>) -> PyResult<Arc<dyn StateStore>> {
    if let Ok(m) = obj.extract::<PyRef<PyMemoryStore>>() {
        return Ok(m.store());
    }
    if let Ok(j) = obj.extract::<PyRef<PyJsonFileStore>>() {
        return Ok(j.store());
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "state_store must be MemoryStore or JsonFileStore",
    ))
}

pub(crate) fn register_state_stores(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMemoryStore>()?;
    m.add_class::<PyJsonFileStore>()?;
    Ok(())
}
