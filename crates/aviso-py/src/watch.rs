// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `PyO3` wrapper for `aviso::watch::WatchRequest` and the supporting types.

use std::collections::BTreeMap;

use aviso::watch::{ResumeStart, WatchMode, WatchRequest};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyInt};

use crate::triggers::PyTrigger;
use crate::values::validate_identifier;

#[pyclass(name = "WatchRequest", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyWatchRequest {
    inner: WatchRequest,
    /// Function triggers, which never reach the core; see `crate::triggers`.
    functions: Vec<crate::triggers::FunctionTrigger>,
}

#[pymethods]
impl PyWatchRequest {
    #[staticmethod]
    fn watch(event_type: String) -> Self {
        Self {
            inner: WatchRequest::watch(event_type),
            functions: Vec::new(),
        }
    }

    #[staticmethod]
    #[pyo3(signature = (event_type, start_from))]
    fn watch_from(event_type: String, start_from: &Bound<'_, PyAny>) -> PyResult<Self> {
        let resume = parse_resume_start(start_from)?;
        Ok(Self {
            inner: WatchRequest::watch_from(event_type, resume),
            functions: Vec::new(),
        })
    }

    #[staticmethod]
    #[pyo3(signature = (event_type, start_from))]
    fn replay_only(event_type: String, start_from: &Bound<'_, PyAny>) -> PyResult<Self> {
        let resume = parse_resume_start(start_from)?;
        Ok(Self {
            inner: WatchRequest::replay_only(event_type, resume),
            functions: Vec::new(),
        })
    }

    fn with_filter(&self, filter: &Bound<'_, PyDict>) -> PyResult<Self> {
        validate_identifier(filter.as_any())?;
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (k, v) in filter {
            let key: String = k.extract()?;
            let value: serde_json::Value = pythonize::depythonize(&v)?;
            map.insert(key, value);
        }
        Ok(Self {
            inner: self.inner.clone().with_filter(map),
            functions: self.functions.clone(),
        })
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "PyO3 cannot extract a borrowed Vec<PyRef<...>> from a Python list; \
                  taking by value is the canonical pattern"
    )]
    fn with_triggers(&self, triggers: Vec<PyRef<'_, PyTrigger>>) -> Self {
        let mut split = crate::triggers::SplitTriggers::default();
        for t in &triggers {
            split.push(t);
        }
        Self {
            inner: self.inner.clone().with_triggers(split.native),
            functions: split.functions,
        }
    }

    #[getter]
    fn event_type(&self) -> &str {
        self.inner.event_type()
    }

    #[getter]
    fn mode(&self) -> &'static str {
        match self.inner.mode() {
            WatchMode::ReplayOnly => "replay_only",
            _ => "watch",
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "WatchRequest(event_type={:?}, mode={:?})",
            self.inner.event_type(),
            self.mode()
        )
    }
}

impl PyWatchRequest {
    pub(crate) fn into_spec(self) -> crate::requests::RequestSpec {
        crate::requests::RequestSpec {
            request: self.inner,
            functions: self.functions,
        }
    }
}

pub(crate) fn parse_resume_start(value: &Bound<'_, PyAny>) -> PyResult<ResumeStart> {
    if let Ok(b) = value.extract::<bool>() {
        return Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "start_from must be an int sequence or a string date, got bool ({b})"
        )));
    }
    if value.is_instance_of::<PyInt>() {
        if let Ok(n) = value.extract::<u64>() {
            return Ok(ResumeStart::AfterSequence(n));
        }
        if let Ok(n) = value.extract::<i128>() {
            if n < 0 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "start_from sequence must be non-negative",
                ));
            }
            return u64::try_from(n)
                .map(ResumeStart::AfterSequence)
                .map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err("start_from sequence exceeds u64::MAX")
                });
        }
        return Err(pyo3::exceptions::PyValueError::new_err(
            "start_from sequence is too large; must fit in u64 (0..=18446744073709551615)",
        ));
    }
    if let Ok(s) = value.extract::<String>() {
        return Ok(ResumeStart::Date(s));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "start_from must be an int sequence or a string date",
    ))
}

pub(crate) fn register_watch(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWatchRequest>()?;
    Ok(())
}
