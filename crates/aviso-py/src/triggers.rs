//! `PyO3` wrapper around `aviso::watch::Trigger` and its six kind constructors.

use aviso::watch::{HttpMethod, Trigger};
use pyo3::prelude::*;

use crate::error::duration_from_seconds;
use crate::paths::normalize_path;

#[pyclass(name = "Trigger", module = "aviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTrigger {
    inner: Trigger,
}

#[pymethods]
impl PyTrigger {
    #[staticmethod]
    #[pyo3(signature = (*, retries = 0, required = true, label = None))]
    fn echo(retries: u32, required: bool, label: Option<String>) -> Self {
        let mut t = Trigger::echo();
        t = t.retries(retries).required(required);
        if let Some(name) = label {
            t = t.label(name);
        }
        Self { inner: t }
    }

    #[staticmethod]
    #[pyo3(signature = (path, *, retries = 0, required = true))]
    fn log(
        py: Python<'_>,
        path: &Bound<'_, PyAny>,
        retries: u32,
        required: bool,
    ) -> PyResult<Self> {
        let p = normalize_path(py, path)?;
        let mut t = Trigger::log(p);
        t = t.retries(retries).required(required);
        Ok(Self { inner: t })
    }

    #[cfg(unix)]
    #[staticmethod]
    #[pyo3(signature = (cmd, *, env = None, working_dir = None, retries = 0, required = true, timeout = None, fail_fast = true))]
    #[allow(clippy::too_many_arguments)]
    fn command(
        py: Python<'_>,
        cmd: String,
        env: Option<std::collections::HashMap<String, String>>,
        working_dir: Option<&Bound<'_, PyAny>>,
        retries: u32,
        required: bool,
        timeout: Option<f64>,
        fail_fast: bool,
    ) -> PyResult<Self> {
        let mut t = Trigger::command(cmd);
        if let Some(envs) = env {
            for (k, v) in envs {
                t = t.env(k, v);
            }
        }
        if let Some(p) = working_dir {
            let wd = normalize_path(py, p)?;
            t = t.working_dir(wd);
        }
        t = t.retries(retries).required(required).fail_fast(fail_fast);
        if let Some(secs) = timeout {
            t = t.timeout(duration_from_seconds("timeout", secs)?);
        }
        Ok(Self { inner: t })
    }

    #[cfg(not(unix))]
    #[staticmethod]
    #[pyo3(signature = (cmd, **kwargs))]
    fn command(cmd: String, kwargs: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<Self> {
        let _ = (cmd, kwargs);
        Err(crate::error::ConfigError::new_err(
            "Trigger.command is Unix-only; not supported on this platform",
        ))
    }

    #[staticmethod]
    #[pyo3(signature = (url, *, method = None, headers = None, body_template = None, retries = 0, required = true, timeout = 30.0, fail_fast = true))]
    #[allow(clippy::too_many_arguments)]
    fn webhook(
        url: String,
        method: Option<String>,
        headers: Option<std::collections::HashMap<String, String>>,
        body_template: Option<String>,
        retries: u32,
        required: bool,
        timeout: f64,
        fail_fast: bool,
    ) -> PyResult<Self> {
        let mut t = Trigger::webhook(url);
        if let Some(m) = method {
            t = t.method(parse_method(&m)?);
        }
        if let Some(hs) = headers {
            for (k, v) in hs {
                t = t.header(k, v);
            }
        }
        if let Some(body) = body_template {
            t = t.body_template(body);
        }
        t = t
            .retries(retries)
            .required(required)
            .fail_fast(fail_fast)
            .timeout(duration_from_seconds("timeout", timeout)?);
        Ok(Self { inner: t })
    }

    #[staticmethod]
    #[pyo3(signature = (url, *, retries = 0, required = true, timeout = 30.0, fail_fast = true))]
    fn teams(
        url: String,
        retries: u32,
        required: bool,
        timeout: f64,
        fail_fast: bool,
    ) -> PyResult<Self> {
        let t = Trigger::teams(url)
            .retries(retries)
            .required(required)
            .fail_fast(fail_fast)
            .timeout(duration_from_seconds("timeout", timeout)?);
        Ok(Self { inner: t })
    }

    #[staticmethod]
    #[pyo3(signature = (url, *, retries = 0, required = true, timeout = 30.0, fail_fast = true))]
    fn post(
        url: String,
        retries: u32,
        required: bool,
        timeout: f64,
        fail_fast: bool,
    ) -> PyResult<Self> {
        let t = Trigger::post(url)
            .retries(retries)
            .required(required)
            .fail_fast(fail_fast)
            .timeout(duration_from_seconds("timeout", timeout)?);
        Ok(Self { inner: t })
    }

    fn retries(&self, n: u32) -> Self {
        Self {
            inner: self.inner.clone().retries(n),
        }
    }

    fn required(&self, on: bool) -> Self {
        Self {
            inner: self.inner.clone().required(on),
        }
    }

    fn timeout(&self, seconds: f64) -> PyResult<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .timeout(duration_from_seconds("timeout", seconds)?),
        })
    }

    fn fail_fast(&self, on: bool) -> Self {
        Self {
            inner: self.inner.clone().fail_fast(on),
        }
    }

    fn label(&self, name: String) -> Self {
        Self {
            inner: self.inner.clone().label(name),
        }
    }

    fn __repr__(&self) -> String {
        format!("Trigger({:?})", self.inner)
    }
}

impl PyTrigger {
    #[allow(
        dead_code,
        reason = "consumed by WatchRequest.with_triggers which lands in the listen commit"
    )]
    pub(crate) fn into_inner(self) -> Trigger {
        self.inner
    }
}

fn parse_method(value: &str) -> PyResult<HttpMethod> {
    match value {
        "POST" => Ok(HttpMethod::Post),
        "GET" => Ok(HttpMethod::Get),
        "PUT" => Ok(HttpMethod::Put),
        "PATCH" => Ok(HttpMethod::Patch),
        "DELETE" => Ok(HttpMethod::Delete),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown HTTP method {other:?}; expected POST/GET/PUT/PATCH/DELETE"
        ))),
    }
}

pub(crate) fn register_triggers(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyTrigger>()?;
    Ok(())
}
