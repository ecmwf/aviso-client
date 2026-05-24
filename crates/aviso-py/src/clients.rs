//! `PyO3` wrapper for the synchronous client.
//!
//! `AvisoClient` exposes the publish and schema-discovery methods of the
//! Rust core to Python users. Methods drive the async core via
//! `runtime().block_on(...)` wrapped in `py.detach` so the GIL is
//! released during the network round-trip and other Python threads can
//! make progress.
//!
//! The async client (`AsyncAvisoClient`) and the watch / listen surface
//! land in subsequent commits.

use std::collections::BTreeMap;

use std::sync::Arc;

use aviso::auth::Bearer;
use aviso::{AvisoClient, NotificationRequest};
use pyo3::prelude::*;

use crate::error::map_client_error;
use crate::runtime::runtime;
use crate::values::{PyNotifyResponse, PySchemaCatalog, PySchemaResponse};

/// Synchronous `PyO3` client. Methods block the current Python thread
/// while the underlying async future runs on the shared tokio runtime.
#[pyclass(name = "AvisoClient", module = "aviso._native", skip_from_py_object)]
pub(crate) struct PyAvisoClient {
    inner: AvisoClient,
}

#[pymethods]
impl PyAvisoClient {
    #[new]
    #[pyo3(signature = (*, base_url, token = None, timeout = None, user_agent = None))]
    fn new(
        base_url: String,
        token: Option<String>,
        timeout: Option<f64>,
        user_agent: Option<String>,
    ) -> PyResult<Self> {
        Python::attach(|py| {
            let mut builder = AvisoClient::builder().base_url(base_url);
            if let Some(t) = token {
                let bearer = Bearer::new(t).map_err(|e| map_client_error(py, e))?;
                builder = builder.auth(Arc::new(bearer));
            }
            if let Some(secs) = timeout {
                builder = builder.timeout(std::time::Duration::from_secs_f64(secs));
            }
            if let Some(ua) = user_agent {
                builder = builder.user_agent(ua);
            }
            let client = builder.build().map_err(|e| map_client_error(py, e))?;
            Ok(Self { inner: client })
        })
    }

    #[getter]
    fn base_url(&self) -> String {
        self.inner.base_url().to_string()
    }

    #[pyo3(signature = (*, event_type, identifier = None, payload = None))]
    fn notify(
        &self,
        py: Python<'_>,
        event_type: String,
        identifier: Option<BTreeMap<String, String>>,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyNotifyResponse> {
        let payload_value: Option<serde_json::Value> = match payload {
            Some(obj) => Some(pythonize::depythonize(obj)?),
            None => None,
        };
        let mut request = NotificationRequest::new(event_type);
        if let Some(id) = identifier {
            request = request.with_identifier(id);
        }
        if let Some(value) = payload_value {
            request = request.with_payload(value);
        }
        let client = self.inner.clone();
        let result = py.detach(|| runtime().block_on(async move { client.notify(&request).await }));
        match result {
            Ok(response) => Ok(PyNotifyResponse::from_core(response)),
            Err(e) => Err(map_client_error(py, e)),
        }
    }

    fn schema(&self, py: Python<'_>) -> PyResult<PySchemaCatalog> {
        let client = self.inner.clone();
        let result = py.detach(|| runtime().block_on(async move { client.schema().await }));
        match result {
            Ok(catalog) => Ok(PySchemaCatalog::from_core(catalog)),
            Err(e) => Err(map_client_error(py, e)),
        }
    }

    fn schema_for(&self, py: Python<'_>, event_type: String) -> PyResult<PySchemaResponse> {
        let client = self.inner.clone();
        let result =
            py.detach(|| runtime().block_on(async move { client.schema_for(&event_type).await }));
        match result {
            Ok(response) => Ok(PySchemaResponse::from_core(response)),
            Err(e) => Err(map_client_error(py, e)),
        }
    }

    #[pyo3(signature = (stream_name))]
    fn wipe_stream(&self, py: Python<'_>, stream_name: String) -> PyResult<()> {
        let client = self.inner.clone();
        let result =
            py.detach(|| runtime().block_on(async move { client.wipe_stream(&stream_name).await }));
        result.map_err(|e| map_client_error(py, e))
    }

    fn wipe_all(&self, py: Python<'_>) -> PyResult<()> {
        let client = self.inner.clone();
        let result = py.detach(|| runtime().block_on(async move { client.wipe_all().await }));
        result.map_err(|e| map_client_error(py, e))
    }

    fn delete_notification(&self, py: Python<'_>, notification_id: String) -> PyResult<()> {
        let client = self.inner.clone();
        let result = py.detach(|| {
            runtime().block_on(async move { client.delete_notification(&notification_id).await })
        });
        result.map_err(|e| map_client_error(py, e))
    }

    fn __repr__(&self) -> String {
        format!("AvisoClient(base_url={:?})", self.inner.base_url().as_str())
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    #[pyo3(signature = (exc_type = None, exc_value = None, traceback = None))]
    #[allow(
        clippy::unused_self,
        reason = "Python's context manager protocol passes the instance to __exit__; \
                  we accept it for shape compliance but the sync client has no \
                  teardown state in this commit"
    )]
    fn __exit__(
        &self,
        exc_type: Option<&Bound<'_, PyAny>>,
        exc_value: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        let _ = (exc_type, exc_value, traceback);
        false
    }
}

pub(crate) fn register_clients(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyAvisoClient>()?;
    Ok(())
}
