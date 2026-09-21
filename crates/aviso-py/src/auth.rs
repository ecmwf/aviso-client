// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `PyO3` wrappers for the five shipped `AuthProvider` implementations, plus
//! `Anonymous`, which is a marker rather than a provider: passing it to a
//! client turns off credential discovery and sends no `Authorization` header.

#![allow(
    clippy::unused_self,
    reason = "Python's __repr__ protocol passes &self by convention; the auth wrappers \
              return constant marker strings so the method does not read self, but the \
              signature is fixed by the protocol"
)]

use std::sync::Arc;

use aviso::auth::{AuthProvider, Basic, Bearer, Chain, ConfigFile, Env};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::error::map_client_error;

/// Thin Python wrapper around `aviso::auth::Bearer`.
#[pyclass(name = "Bearer", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyBearer {
    inner: Arc<dyn AuthProvider>,
}

#[pymethods]
impl PyBearer {
    #[new]
    fn new(py: Python<'_>, token: String) -> PyResult<Self> {
        let provider = Bearer::new(token).map_err(|e| map_client_error(py, e))?;
        Ok(Self {
            inner: Arc::new(provider),
        })
    }

    fn __repr__(&self) -> &'static str {
        "Bearer(token=<redacted>)"
    }
}

impl PyBearer {
    pub(crate) fn provider(&self) -> Arc<dyn AuthProvider> {
        Arc::clone(&self.inner)
    }
}

/// Thin Python wrapper around `aviso::auth::Basic`.
#[pyclass(name = "Basic", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyBasic {
    inner: Arc<dyn AuthProvider>,
}

#[pymethods]
impl PyBasic {
    #[new]
    #[pyo3(signature = (username, password = String::new()))]
    fn new(py: Python<'_>, username: String, password: String) -> PyResult<Self> {
        let provider = Basic::new(username, password).map_err(|e| map_client_error(py, e))?;
        Ok(Self {
            inner: Arc::new(provider),
        })
    }

    fn __repr__(&self) -> &'static str {
        "Basic(username=...,password=<redacted>)"
    }
}

impl PyBasic {
    pub(crate) fn provider(&self) -> Arc<dyn AuthProvider> {
        Arc::clone(&self.inner)
    }
}

/// Thin Python wrapper around `aviso::auth::Env`.
#[pyclass(name = "Env", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyEnv {
    inner: Arc<dyn AuthProvider>,
}

#[pymethods]
impl PyEnv {
    #[new]
    fn new(py: Python<'_>) -> PyResult<Self> {
        let provider = Env::from_process_env().map_err(|e| map_client_error(py, e))?;
        Ok(Self {
            inner: Arc::new(provider),
        })
    }

    fn __repr__(&self) -> &'static str {
        "Env(source=process-env)"
    }
}

impl PyEnv {
    pub(crate) fn provider(&self) -> Arc<dyn AuthProvider> {
        Arc::clone(&self.inner)
    }
}

/// Thin Python wrapper around `aviso::auth::ConfigFile`.
#[pyclass(name = "ConfigFile", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyConfigFile {
    inner: Arc<dyn AuthProvider>,
}

#[pymethods]
impl PyConfigFile {
    #[new]
    fn new(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        let path_buf = crate::paths::normalize_path(py, path)?;
        let provider = ConfigFile::from_path(&path_buf).map_err(|e| map_client_error(py, e))?;
        Ok(Self {
            inner: Arc::new(provider),
        })
    }

    fn __repr__(&self) -> &'static str {
        "ConfigFile(...)"
    }
}

impl PyConfigFile {
    pub(crate) fn provider(&self) -> Arc<dyn AuthProvider> {
        Arc::clone(&self.inner)
    }
}

/// Thin Python wrapper around `aviso::auth::Chain`.
#[pyclass(name = "Chain", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyChain {
    inner: Arc<dyn AuthProvider>,
}

#[pymethods]
impl PyChain {
    #[new]
    #[pyo3(signature = (*args))]
    fn new(args: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
        for obj in args.iter() {
            providers.push(extract_provider(&obj)?);
        }
        Ok(Self {
            inner: Arc::new(Chain::new(providers)),
        })
    }

    fn __repr__(&self) -> String {
        "Chain(...)".to_string()
    }
}

impl PyChain {
    pub(crate) fn provider(&self) -> Arc<dyn AuthProvider> {
        Arc::clone(&self.inner)
    }
}

/// Marker that turns off credential discovery.
///
/// A client built with `auth=Anonymous()` sends no `Authorization` header even
/// when a credential is sitting in the environment or in a file. It carries no
/// credential of its own, so it is not an `AuthProvider` and cannot go into a
/// `Chain`.
#[pyclass(name = "Anonymous", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyAnonymous;

#[pymethods]
impl PyAnonymous {
    #[new]
    const fn new() -> Self {
        Self
    }

    fn __repr__(&self) -> &'static str {
        "Anonymous()"
    }
}

/// True when the object is the [`PyAnonymous`] marker.
pub(crate) fn is_anonymous(obj: &Bound<'_, PyAny>) -> bool {
    obj.extract::<PyRef<PyAnonymous>>().is_ok()
}

/// Extracts an `Arc<dyn AuthProvider>` from any of the five shipped Python
/// wrapper classes. Used by `Chain.__init__` and `AvisoClient(auth=...)`.
pub(crate) fn extract_provider(obj: &Bound<'_, PyAny>) -> PyResult<Arc<dyn AuthProvider>> {
    if let Ok(b) = obj.extract::<PyRef<PyBearer>>() {
        return Ok(b.provider());
    }
    if let Ok(b) = obj.extract::<PyRef<PyBasic>>() {
        return Ok(b.provider());
    }
    if let Ok(b) = obj.extract::<PyRef<PyEnv>>() {
        return Ok(b.provider());
    }
    if let Ok(b) = obj.extract::<PyRef<PyConfigFile>>() {
        return Ok(b.provider());
    }
    if let Ok(b) = obj.extract::<PyRef<PyChain>>() {
        return Ok(b.provider());
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "auth must be one of Bearer / Basic / Env / ConfigFile / Chain / Anonymous",
    ))
}

pub(crate) fn register_auth(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBearer>()?;
    m.add_class::<PyBasic>()?;
    m.add_class::<PyEnv>()?;
    m.add_class::<PyConfigFile>()?;
    m.add_class::<PyChain>()?;
    m.add_class::<PyAnonymous>()?;
    Ok(())
}
