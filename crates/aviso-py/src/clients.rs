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

use aviso::{AvisoClient, NotificationRequest};
use pyo3::prelude::*;

use crate::auth::extract_provider;
use crate::error::map_client_error;
use crate::runtime::runtime;
use crate::state_stores::extract_store;
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
    #[pyo3(signature = (*, base_url, auth = None, timeout = None, user_agent = None,
                          state_store = None, heartbeat_interval = None,
                          danger_accept_invalid_certs = false,
                          flush_cursor_on_exit = false))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        base_url: String,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<f64>,
        user_agent: Option<String>,
        state_store: Option<&Bound<'_, PyAny>>,
        heartbeat_interval: Option<f64>,
        danger_accept_invalid_certs: bool,
        flush_cursor_on_exit: bool,
    ) -> PyResult<Self> {
        let mut builder = AvisoClient::builder().base_url(base_url);
        if let Some(provider) = auth {
            builder = builder.auth(extract_provider(provider)?);
        }
        if let Some(secs) = timeout {
            builder = builder.timeout(std::time::Duration::from_secs_f64(secs));
        }
        if let Some(ua) = user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(store) = state_store {
            builder = builder.state_store(extract_store(store)?);
        }
        if let Some(secs) = heartbeat_interval {
            builder = builder.heartbeat_interval(std::time::Duration::from_secs_f64(secs));
        }
        if danger_accept_invalid_certs {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if flush_cursor_on_exit {
            builder = builder.flush_cursor_on_exit(true);
        }
        let client = builder.build().map_err(|e| map_client_error(py, e))?;
        Ok(Self { inner: client })
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
