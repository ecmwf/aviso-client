// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The settings a client resolved, exposed to Python as `ResolvedConfig`.
//!
//! A client built with no arguments takes its server address, timeouts and
//! credential from the environment and the aviso config file. `client.config`
//! and `pyaviso.resolve_config()` show which values it ended up with and
//! where each came from, without the secret: the credential is described by
//! its kind and source, and the address has any `user:password@` removed. The
//! `repr()` is written to be pasted into a ticket.

use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use aviso::resolve::{ResolvedSettings, Source};

/// The value of one setting together with where it came from.
#[pyclass(
    name = "SourcedValue",
    module = "pyaviso._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PySourcedValue {
    #[pyo3(get)]
    value: Py<PyAny>,
    #[pyo3(get)]
    source: String,
}

#[pymethods]
impl PySourcedValue {
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "SourcedValue(value={}, source={:?})",
            self.value.bind(py).repr()?,
            self.source
        ))
    }
}

/// The credential a client sends, described without the secret.
#[pyclass(
    name = "ResolvedAuth",
    module = "pyaviso._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyResolvedAuth {
    /// `bearer`, `basic`, or a custom provider's own name.
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    source: String,
    /// Why the credential is not being sent, when it is not.
    #[pyo3(get)]
    refused: Option<String>,
}

#[pymethods]
impl PyResolvedAuth {
    fn __repr__(&self) -> String {
        match &self.refused {
            Some(why) => format!(
                "ResolvedAuth(kind={:?}, source={:?}, refused={why:?})",
                self.kind, self.source
            ),
            None => format!(
                "ResolvedAuth(kind={:?}, source={:?})",
                self.kind, self.source
            ),
        }
    }
}

/// Everything a client's connection depends on, each with its source.
#[pyclass(
    name = "ResolvedConfig",
    module = "pyaviso._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyResolvedConfig {
    /// The server address with credentials removed, or `None` when no source
    /// supplied one.
    #[pyo3(get)]
    base_url: Option<Py<PySourcedValue>>,
    /// Per-request timeout in seconds, or `None` for the library default.
    #[pyo3(get)]
    timeout: Py<PySourcedValue>,
    /// Expected heartbeat interval in seconds, or `None` for the default.
    #[pyo3(get)]
    heartbeat_interval: Py<PySourcedValue>,
    /// Extra CA bundle paths.
    #[pyo3(get)]
    ca_bundle: Py<PySourcedValue>,
    /// Whether certificate validation is off.
    #[pyo3(get)]
    danger_accept_invalid_certs: Py<PySourcedValue>,
    /// The credential, or `None` for anonymous access.
    #[pyo3(get)]
    auth: Option<Py<PyResolvedAuth>>,
    /// The config file that was read, if one existed.
    #[pyo3(get)]
    config_file: Option<String>,
    /// The credentials file that was consulted, if any.
    #[pyo3(get)]
    credentials_file: Option<String>,
}

#[pymethods]
impl PyResolvedConfig {
    /// A plain dict of the same information, for logging as JSON.
    fn as_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let field = |v: &Py<PySourcedValue>| -> PyResult<Bound<'py, PyDict>> {
            let inner = PyDict::new(py);
            let v = v.borrow(py);
            inner.set_item("value", v.value.bind(py))?;
            inner.set_item("source", &v.source)?;
            Ok(inner)
        };
        d.set_item("base_url", self.base_url.as_ref().map(field).transpose()?)?;
        d.set_item("timeout", field(&self.timeout)?)?;
        d.set_item("heartbeat_interval", field(&self.heartbeat_interval)?)?;
        d.set_item("ca_bundle", field(&self.ca_bundle)?)?;
        d.set_item(
            "danger_accept_invalid_certs",
            field(&self.danger_accept_invalid_certs)?,
        )?;
        match &self.auth {
            Some(a) => {
                let a = a.borrow(py);
                let inner = PyDict::new(py);
                inner.set_item("kind", &a.kind)?;
                inner.set_item("source", &a.source)?;
                inner.set_item("refused", a.refused.as_deref())?;
                d.set_item("auth", inner)?;
            }
            None => d.set_item("auth", py.None())?,
        }
        d.set_item("config_file", self.config_file.as_deref())?;
        d.set_item("credentials_file", self.credentials_file.as_deref())?;
        Ok(d)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let mut lines = vec!["ResolvedConfig(".to_string()];
        let mut push = |name: &str, value: String, source: &str| {
            let assignment = format!("{name}={value}");
            lines.push(format!("    {assignment:<48} ({source}),"));
        };
        match &self.base_url {
            Some(v) => {
                let v = v.borrow(py);
                push("base_url", v.value.bind(py).repr()?.to_string(), &v.source);
            }
            None => push(
                "base_url",
                "None".into(),
                "not set in code, AVISO_BASE_URL or the config file",
            ),
        }
        match &self.auth {
            Some(a) => {
                let a = a.borrow(py);
                let source = match &a.refused {
                    Some(why) => format!("{}; {why}", a.source),
                    None => a.source.clone(),
                };
                push("auth", format!("{:?}", a.kind), &source);
            }
            None => push("auth", "None".into(), "anonymous"),
        }
        for (name, v) in [
            ("timeout", &self.timeout),
            ("heartbeat_interval", &self.heartbeat_interval),
            ("ca_bundle", &self.ca_bundle),
            (
                "danger_accept_invalid_certs",
                &self.danger_accept_invalid_certs,
            ),
        ] {
            let v = v.borrow(py);
            push(name, v.value.bind(py).repr()?.to_string(), &v.source);
        }
        lines.push(")".to_string());
        Ok(lines.join("\n"))
    }
}

fn sourced<'py, T>(py: Python<'py>, value: T, source: &Source) -> PyResult<Py<PySourcedValue>>
where
    T: IntoPyObject<'py>,
{
    Py::new(
        py,
        PySourcedValue {
            value: value.into_py_any(py)?,
            source: source_label(source),
        },
    )
}

fn source_label(source: &Source) -> String {
    source.to_string()
}

fn secs(d: Option<std::time::Duration>) -> Option<f64> {
    d.map(|d| d.as_secs_f64())
}

/// Converts the core report into the Python object.
pub(crate) fn to_py(py: Python<'_>, r: &ResolvedSettings) -> PyResult<Py<PyResolvedConfig>> {
    let base_url = match &r.base_url {
        Some(s) => Some(sourced(py, s.value.clone(), &s.source)?),
        None => None,
    };
    let auth = match &r.auth {
        Some(a) => Some(Py::new(
            py,
            PyResolvedAuth {
                kind: a.kind.to_string(),
                source: source_label(&a.source),
                refused: a.refused.clone(),
            },
        )?),
        None => None,
    };
    Py::new(
        py,
        PyResolvedConfig {
            base_url,
            timeout: sourced(py, secs(r.timeout.value), &r.timeout.source)?,
            heartbeat_interval: sourced(
                py,
                secs(r.heartbeat_interval.value),
                &r.heartbeat_interval.source,
            )?,
            ca_bundle: sourced(
                py,
                r.ca_bundle
                    .value
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
                &r.ca_bundle.source,
            )?,
            danger_accept_invalid_certs: sourced(
                py,
                r.danger_accept_invalid_certs.value,
                &r.danger_accept_invalid_certs.source,
            )?,
            auth,
            config_file: r.config_file.as_ref().map(|p| p.display().to_string()),
            credentials_file: r.credentials_file.as_ref().map(|p| p.display().to_string()),
        },
    )
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySourcedValue>()?;
    m.add_class::<PyResolvedAuth>()?;
    m.add_class::<PyResolvedConfig>()?;
    Ok(())
}
