// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `PyO3` wrappers for the synchronous and asynchronous clients.
//!
//! `AvisoClient` exposes the publish, schema-discovery, admin, and listen
//! methods of the Rust core to Python users. Methods drive the async core
//! via `runtime().block_on(...)` wrapped in `py.detach` so the GIL is
//! released during the network round-trip and other Python threads can
//! make progress. `AsyncAvisoClient` exposes the same surface returning
//! Python awaitables via `pyo3_async_runtimes::tokio::future_into_py`,
//! scheduled on the same shared runtime.

use std::collections::BTreeMap;

use aviso::{AvisoClient, NotificationRequest};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::client_args::{Built, ClientArgs};
use crate::error::map_client_error;
use crate::requests::build_watch_request;
use crate::runtime::runtime;
use crate::streams::{PyAsyncNotificationIterator, PyNotificationIterator};
use crate::values::{
    PyNotifyResponse, PyNotifyResult, PySchemaCatalog, PySchemaResponse, validate_identifier,
};
use crate::watch::PyWatchRequest;

fn identifier_from_py(
    obj: &Bound<'_, PyAny>,
    error_message: impl FnOnce() -> String,
) -> PyResult<BTreeMap<String, serde_json::Value>> {
    validate_identifier(obj)?;
    pythonize::depythonize(obj).map_err(|_| PyTypeError::new_err(error_message()))
}

fn request_from_mapping(index: usize, obj: &Bound<'_, PyAny>) -> PyResult<NotificationRequest> {
    let dict = obj.cast::<PyDict>().map_err(|_| {
        PyTypeError::new_err(format!(
            "notifications[{index}] must be a dict with keys event_type, identifier, payload"
        ))
    })?;
    let event_type: String = match dict.get_item("event_type")? {
        Some(value) => value.extract().map_err(|_| {
            PyTypeError::new_err(format!(
                "notifications[{index}].event_type must be a string"
            ))
        })?,
        None => {
            return Err(PyValueError::new_err(format!(
                "notifications[{index}] is missing required key 'event_type'"
            )));
        }
    };
    let mut request = NotificationRequest::new(event_type);
    if let Some(identifier) = dict.get_item("identifier")?
        && !identifier.is_none()
    {
        let map = identifier_from_py(&identifier, || {
            format!("notifications[{index}].identifier must be a dict of str to JSON values")
        })?;
        request = request.with_identifier(map);
    }
    if let Some(payload) = dict.get_item("payload")?
        && !payload.is_none()
    {
        let value: serde_json::Value = pythonize::depythonize(&payload)?;
        request = request.with_payload(value);
    }
    Ok(request)
}

fn requests_from_pylist(
    notifications: Vec<Bound<'_, PyAny>>,
) -> PyResult<Vec<NotificationRequest>> {
    notifications
        .into_iter()
        .enumerate()
        .map(|(index, obj)| request_from_mapping(index, &obj))
        .collect()
}

fn resolve_concurrency(concurrency: i64) -> PyResult<usize> {
    usize::try_from(concurrency).map_err(|_| {
        PyValueError::new_err(format!(
            "concurrency must be non-negative; got {concurrency}"
        ))
    })
}

fn build_results(
    py: Python<'_>,
    results: Vec<aviso::Result<aviso::NotifyResponse>>,
) -> PyResult<Vec<Py<PyNotifyResult>>> {
    results
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            let item = match result {
                Ok(response) => PyNotifyResult::success(py, index, response)?,
                Err(err) => PyNotifyResult::failure(py, index, map_client_error(py, err)),
            };
            Py::new(py, item)
        })
        .collect()
}

/// Synchronous `PyO3` client. Methods block the current Python thread
/// while the underlying async future runs on the shared tokio runtime.
#[pyclass(name = "AvisoClient", module = "pyaviso._native", skip_from_py_object)]
pub(crate) struct PyAvisoClient {
    inner: AvisoClient,
    config: Py<crate::config::PyResolvedConfig>,
}

#[pymethods]
impl PyAvisoClient {
    /// Builds a client. Every argument is optional: what is not given is
    /// taken from the environment, then the aviso config file, then the
    /// library default. `client.config` shows what was chosen and from where.
    #[new]
    #[pyo3(signature = (*, base_url = None, auth = None, timeout = None, user_agent = None,
                          state_store = None, heartbeat_interval = None,
                          danger_accept_invalid_certs = None,
                          flush_cursor_on_exit = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        base_url: Option<String>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<f64>,
        user_agent: Option<String>,
        state_store: Option<&Bound<'_, PyAny>>,
        heartbeat_interval: Option<f64>,
        danger_accept_invalid_certs: Option<bool>,
        flush_cursor_on_exit: Option<bool>,
    ) -> PyResult<Self> {
        let args = ClientArgs {
            base_url,
            auth,
            timeout,
            user_agent,
            state_store,
            heartbeat_interval,
            danger_accept_invalid_certs,
            flush_cursor_on_exit,
        };
        let Built { client, config } = args.build(py)?;
        Ok(Self {
            inner: client,
            config,
        })
    }

    /// The settings this client uses and where each came from. Safe to log:
    /// the credential is described by kind and source, never by value.
    #[getter]
    fn config(&self, py: Python<'_>) -> Py<crate::config::PyResolvedConfig> {
        self.config.clone_ref(py)
    }

    /// Builds a client from the aviso config file, then applies any keyword
    /// arguments given here on top of it.
    #[staticmethod]
    #[pyo3(signature = (path = None, *, base_url = None, auth = None, timeout = None,
                          user_agent = None, state_store = None, heartbeat_interval = None,
                          danger_accept_invalid_certs = None, flush_cursor_on_exit = None))]
    #[allow(clippy::too_many_arguments)]
    fn from_file(
        py: Python<'_>,
        path: Option<&Bound<'_, PyAny>>,
        base_url: Option<String>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<f64>,
        user_agent: Option<String>,
        state_store: Option<&Bound<'_, PyAny>>,
        heartbeat_interval: Option<f64>,
        danger_accept_invalid_certs: Option<bool>,
        flush_cursor_on_exit: Option<bool>,
    ) -> PyResult<Self> {
        let args = ClientArgs {
            base_url,
            auth,
            timeout,
            user_agent,
            state_store,
            heartbeat_interval,
            danger_accept_invalid_certs,
            flush_cursor_on_exit,
        };
        let Built { client, config } = args.build_from_file(py, path)?;
        Ok(Self {
            inner: client,
            config,
        })
    }

    /// The base URL exactly as configured, including any `user:password@`
    /// it carries. This is the value the client uses; `repr()` shows the
    /// same URL without the credentials, and is the one to log.
    #[getter]
    fn base_url(&self) -> String {
        self.inner.base_url().to_string()
    }

    #[pyo3(signature = (*, event_type, identifier = None, payload = None))]
    fn notify(
        &self,
        py: Python<'_>,
        event_type: String,
        identifier: Option<&Bound<'_, PyAny>>,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyNotifyResponse> {
        let payload_value: Option<serde_json::Value> = match payload {
            Some(obj) => Some(pythonize::depythonize(obj)?),
            None => None,
        };
        let mut request = NotificationRequest::new(event_type);
        if let Some(identifier) = identifier {
            request = request.with_identifier(identifier_from_py(identifier, || {
                "identifier must be a mapping from str to JSON values".to_string()
            })?);
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

    #[pyo3(signature = (notifications, *, concurrency = 0))]
    fn notify_many(
        &self,
        py: Python<'_>,
        notifications: Vec<Bound<'_, PyAny>>,
        concurrency: i64,
    ) -> PyResult<Vec<Py<PyNotifyResult>>> {
        let concurrency = resolve_concurrency(concurrency)?;
        let requests = requests_from_pylist(notifications)?;
        let client = self.inner.clone();
        let results = py.detach(|| {
            runtime().block_on(async move { client.notify_many(&requests, concurrency).await })
        });
        build_results(py, results)
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

    #[pyo3(signature = (event_type = None, *, filter = None, start_from = None, mode = None, triggers = None, request = None))]
    #[allow(clippy::too_many_arguments)]
    fn listen(
        &self,
        py: Python<'_>,
        event_type: Option<String>,
        filter: Option<&Bound<'_, PyDict>>,
        start_from: Option<&Bound<'_, PyAny>>,
        mode: Option<&str>,
        triggers: Option<&Bound<'_, PyAny>>,
        request: Option<PyRef<'_, PyWatchRequest>>,
    ) -> PyResult<PyNotificationIterator> {
        let req = build_watch_request(event_type, filter, start_from, mode, triggers, request)?;
        let client = self.inner.clone();
        let stream = py.detach(|| runtime().block_on(async move { client.watch(req) }));
        let stream = stream.map_err(|e| map_client_error(py, e))?;
        Ok(PyNotificationIterator::new(stream))
    }

    fn __repr__(&self) -> String {
        format!("AvisoClient(base_url={:?})", self.inner.display_base_url())
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

/// Async `PyO3` client. Methods return Python awaitables driven by the
/// shared tokio runtime via `pyo3-async-runtimes`.
#[pyclass(
    name = "AsyncAvisoClient",
    module = "pyaviso._native",
    skip_from_py_object
)]
pub(crate) struct PyAsyncAvisoClient {
    inner: AvisoClient,
    config: Py<crate::config::PyResolvedConfig>,
}

#[pymethods]
impl PyAsyncAvisoClient {
    /// Builds a client. Every argument is optional: what is not given is
    /// taken from the environment, then the aviso config file, then the
    /// library default. `client.config` shows what was chosen and from where.
    #[new]
    #[pyo3(signature = (*, base_url = None, auth = None, timeout = None, user_agent = None,
                          state_store = None, heartbeat_interval = None,
                          danger_accept_invalid_certs = None,
                          flush_cursor_on_exit = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        base_url: Option<String>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<f64>,
        user_agent: Option<String>,
        state_store: Option<&Bound<'_, PyAny>>,
        heartbeat_interval: Option<f64>,
        danger_accept_invalid_certs: Option<bool>,
        flush_cursor_on_exit: Option<bool>,
    ) -> PyResult<Self> {
        let args = ClientArgs {
            base_url,
            auth,
            timeout,
            user_agent,
            state_store,
            heartbeat_interval,
            danger_accept_invalid_certs,
            flush_cursor_on_exit,
        };
        let Built { client, config } = args.build(py)?;
        Ok(Self {
            inner: client,
            config,
        })
    }

    /// The settings this client uses and where each came from. Safe to log:
    /// the credential is described by kind and source, never by value.
    #[getter]
    fn config(&self, py: Python<'_>) -> Py<crate::config::PyResolvedConfig> {
        self.config.clone_ref(py)
    }

    /// Builds a client from the aviso config file, then applies any keyword
    /// arguments given here on top of it.
    #[staticmethod]
    #[pyo3(signature = (path = None, *, base_url = None, auth = None, timeout = None,
                          user_agent = None, state_store = None, heartbeat_interval = None,
                          danger_accept_invalid_certs = None, flush_cursor_on_exit = None))]
    #[allow(clippy::too_many_arguments)]
    fn from_file(
        py: Python<'_>,
        path: Option<&Bound<'_, PyAny>>,
        base_url: Option<String>,
        auth: Option<&Bound<'_, PyAny>>,
        timeout: Option<f64>,
        user_agent: Option<String>,
        state_store: Option<&Bound<'_, PyAny>>,
        heartbeat_interval: Option<f64>,
        danger_accept_invalid_certs: Option<bool>,
        flush_cursor_on_exit: Option<bool>,
    ) -> PyResult<Self> {
        let args = ClientArgs {
            base_url,
            auth,
            timeout,
            user_agent,
            state_store,
            heartbeat_interval,
            danger_accept_invalid_certs,
            flush_cursor_on_exit,
        };
        let Built { client, config } = args.build_from_file(py, path)?;
        Ok(Self {
            inner: client,
            config,
        })
    }

    /// The base URL exactly as configured, including any `user:password@`
    /// it carries. This is the value the client uses; `repr()` shows the
    /// same URL without the credentials, and is the one to log.
    #[getter]
    fn base_url(&self) -> String {
        self.inner.base_url().to_string()
    }

    #[pyo3(signature = (*, event_type, identifier = None, payload = None))]
    fn notify<'py>(
        &self,
        py: Python<'py>,
        event_type: String,
        identifier: Option<&Bound<'_, PyAny>>,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let payload_value: Option<serde_json::Value> = match payload {
            Some(obj) => Some(pythonize::depythonize(obj)?),
            None => None,
        };
        let mut request = NotificationRequest::new(event_type);
        if let Some(identifier) = identifier {
            request = request.with_identifier(identifier_from_py(identifier, || {
                "identifier must be a mapping from str to JSON values".to_string()
            })?);
        }
        if let Some(value) = payload_value {
            request = request.with_payload(value);
        }
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .notify(&request)
                .await
                .map(PyNotifyResponse::from_core)
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
        })
    }

    #[pyo3(signature = (notifications, *, concurrency = 0))]
    fn notify_many<'py>(
        &self,
        py: Python<'py>,
        notifications: Vec<Bound<'py, PyAny>>,
        concurrency: i64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let concurrency = resolve_concurrency(concurrency)?;
        let requests = requests_from_pylist(notifications)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let results = client.notify_many(&requests, concurrency).await;
            Python::attach(|py| build_results(py, results))
        })
    }

    fn schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .schema()
                .await
                .map(PySchemaCatalog::from_core)
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
        })
    }

    fn schema_for<'py>(&self, py: Python<'py>, event_type: String) -> PyResult<Bound<'py, PyAny>> {
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .schema_for(&event_type)
                .await
                .map(PySchemaResponse::from_core)
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
        })
    }

    fn wipe_stream<'py>(
        &self,
        py: Python<'py>,
        stream_name: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .wipe_stream(&stream_name)
                .await
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
        })
    }

    fn wipe_all<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .wipe_all()
                .await
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
        })
    }

    fn delete_notification<'py>(
        &self,
        py: Python<'py>,
        notification_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .delete_notification(&notification_id)
                .await
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
        })
    }

    #[pyo3(signature = (event_type = None, *, filter = None, start_from = None, mode = None, triggers = None, request = None))]
    #[allow(clippy::too_many_arguments)]
    fn listen(
        &self,
        py: Python<'_>,
        event_type: Option<String>,
        filter: Option<&Bound<'_, PyDict>>,
        start_from: Option<&Bound<'_, PyAny>>,
        mode: Option<&str>,
        triggers: Option<&Bound<'_, PyAny>>,
        request: Option<PyRef<'_, PyWatchRequest>>,
    ) -> PyResult<PyAsyncNotificationIterator> {
        let req = build_watch_request(event_type, filter, start_from, mode, triggers, request)?;
        let client = self.inner.clone();
        let stream = py.detach(|| runtime().block_on(async move { client.watch(req) }));
        let stream = stream.map_err(|e| map_client_error(py, e))?;
        Ok(PyAsyncNotificationIterator::new(stream))
    }

    fn __repr__(&self) -> String {
        format!(
            "AsyncAvisoClient(base_url={:?})",
            self.inner.display_base_url()
        )
    }
}

pub(crate) fn register_clients(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyAvisoClient>()?;
    m.add_class::<PyAsyncAvisoClient>()?;
    m.add_function(wrap_pyfunction!(crate::client_args::resolve_config, m)?)?;
    Ok(())
}
