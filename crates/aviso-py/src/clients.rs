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
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::auth::extract_provider;
use crate::error::{duration_from_seconds, map_client_error};
use crate::runtime::runtime;
use crate::state_stores::extract_store;
use crate::streams::{PyAsyncNotificationIterator, PyNotificationIterator};
use crate::triggers::PyTrigger;
use crate::values::{PyNotifyResponse, PySchemaCatalog, PySchemaResponse};
use crate::watch::{PyWatchRequest, parse_resume_start};

/// Synchronous `PyO3` client. Methods block the current Python thread
/// while the underlying async future runs on the shared tokio runtime.
#[pyclass(name = "AvisoClient", module = "pyaviso._native", skip_from_py_object)]
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
            builder = builder.timeout(duration_from_seconds("timeout", secs)?);
        }
        if let Some(ua) = user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(store) = state_store {
            builder = builder.state_store(extract_store(store)?);
        }
        if let Some(secs) = heartbeat_interval {
            builder =
                builder.heartbeat_interval(duration_from_seconds("heartbeat_interval", secs)?);
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

    #[pyo3(signature = (event_type = None, *, filter = None, from_ = None, mode = None, triggers = None, request = None))]
    #[allow(clippy::too_many_arguments)]
    fn listen(
        &self,
        py: Python<'_>,
        event_type: Option<String>,
        filter: Option<&Bound<'_, PyDict>>,
        from_: Option<&Bound<'_, PyAny>>,
        mode: Option<&str>,
        triggers: Option<&Bound<'_, PyAny>>,
        request: Option<PyRef<'_, PyWatchRequest>>,
    ) -> PyResult<PyNotificationIterator> {
        let req = build_watch_request(event_type, filter, from_, mode, triggers, request)?;
        let client = self.inner.clone();
        let stream = py.detach(|| runtime().block_on(async move { client.watch(req) }));
        let stream = stream.map_err(|e| map_client_error(py, e))?;
        Ok(PyNotificationIterator::new(stream))
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

/// Async `PyO3` client. Methods return Python awaitables driven by the
/// shared tokio runtime via `pyo3-async-runtimes`.
#[pyclass(
    name = "AsyncAvisoClient",
    module = "pyaviso._native",
    skip_from_py_object
)]
pub(crate) struct PyAsyncAvisoClient {
    inner: AvisoClient,
}

#[pymethods]
impl PyAsyncAvisoClient {
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
            builder = builder.timeout(duration_from_seconds("timeout", secs)?);
        }
        if let Some(ua) = user_agent {
            builder = builder.user_agent(ua);
        }
        if let Some(store) = state_store {
            builder = builder.state_store(extract_store(store)?);
        }
        if let Some(secs) = heartbeat_interval {
            builder =
                builder.heartbeat_interval(duration_from_seconds("heartbeat_interval", secs)?);
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
    fn notify<'py>(
        &self,
        py: Python<'py>,
        event_type: String,
        identifier: Option<std::collections::BTreeMap<String, String>>,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
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
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            client
                .notify(&request)
                .await
                .map(PyNotifyResponse::from_core)
                .map_err(|e| Python::attach(|py| map_client_error(py, e)))
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

    #[pyo3(signature = (event_type = None, *, filter = None, from_ = None, mode = None, triggers = None, request = None))]
    #[allow(clippy::too_many_arguments)]
    fn listen(
        &self,
        py: Python<'_>,
        event_type: Option<String>,
        filter: Option<&Bound<'_, PyDict>>,
        from_: Option<&Bound<'_, PyAny>>,
        mode: Option<&str>,
        triggers: Option<&Bound<'_, PyAny>>,
        request: Option<PyRef<'_, PyWatchRequest>>,
    ) -> PyResult<PyAsyncNotificationIterator> {
        let req = build_watch_request(event_type, filter, from_, mode, triggers, request)?;
        let client = self.inner.clone();
        let stream = py.detach(|| runtime().block_on(async move { client.watch(req) }));
        let stream = stream.map_err(|e| map_client_error(py, e))?;
        Ok(PyAsyncNotificationIterator::new(stream))
    }

    fn __repr__(&self) -> String {
        format!(
            "AsyncAvisoClient(base_url={:?})",
            self.inner.base_url().as_str()
        )
    }
}

fn build_watch_request(
    event_type: Option<String>,
    filter: Option<&Bound<'_, PyDict>>,
    from_: Option<&Bound<'_, PyAny>>,
    mode: Option<&str>,
    triggers: Option<&Bound<'_, PyAny>>,
    request: Option<PyRef<'_, PyWatchRequest>>,
) -> PyResult<aviso::watch::WatchRequest> {
    if let Some(req) = request {
        if event_type.is_some() || filter.is_some() || from_.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "request is mutually exclusive with event_type, filter, and from_",
            ));
        }
        if mode.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "request already carries a mode; do not pass mode= when using request=",
            ));
        }
        if triggers.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "triggers= cannot be combined with request=; add triggers to the WatchRequest instead",
            ));
        }
        return Ok(req.clone().into_inner());
    }
    let event = event_type.ok_or_else(|| {
        crate::error::AvisoError::new_err(
            "listen() requires either event_type=<str> or request=<WatchRequest>",
        )
    })?;
    let effective_mode = mode.unwrap_or("watch");
    let mut req = match (effective_mode, from_) {
        ("watch", None) => aviso::watch::WatchRequest::watch(event),
        ("watch", Some(from_obj)) => {
            let resume = parse_resume_start(from_obj)?;
            aviso::watch::WatchRequest::watch_from(event, resume)
        }
        ("replay_only", Some(from_obj)) => {
            let resume = parse_resume_start(from_obj)?;
            aviso::watch::WatchRequest::replay_only(event, resume)
        }
        ("replay_only", None) => {
            return Err(crate::error::AvisoError::new_err(
                "replay_only mode requires from_=<int sequence or date string>",
            ));
        }
        _ => {
            return Err(crate::error::AvisoError::new_err(format!(
                "unknown WatchMode {effective_mode:?}; expected 'watch' or 'replay_only'"
            )));
        }
    };
    if let Some(filter_dict) = filter {
        let mut map = std::collections::BTreeMap::<String, serde_json::Value>::new();
        for (k, v) in filter_dict {
            let key: String = k.extract()?;
            let value: serde_json::Value = pythonize::depythonize(&v)?;
            map.insert(key, value);
        }
        req = req.with_filter(map);
    }
    if let Some(triggers_value) = triggers {
        let trigger_vec = extract_triggers(triggers_value)?;
        if !trigger_vec.is_empty() {
            req = req.with_triggers(trigger_vec);
        }
    }
    Ok(req)
}

fn extract_triggers(value: &Bound<'_, PyAny>) -> PyResult<Vec<aviso::watch::Trigger>> {
    if value.is_instance_of::<PyTrigger>() {
        return Err(crate::error::AvisoError::new_err(
            "triggers= must be a sequence of Trigger; pass [trigger] for a single trigger",
        ));
    }
    if value.is_instance_of::<pyo3::types::PyString>()
        || value.is_instance_of::<pyo3::types::PyBytes>()
    {
        return Err(crate::error::AvisoError::new_err(
            "triggers= must be a sequence of Trigger, not a string or bytes",
        ));
    }
    let iter = value.try_iter().map_err(|_| {
        crate::error::AvisoError::new_err(
            "triggers= must be an iterable of Trigger instances (list or tuple)",
        )
    })?;
    let mut out = Vec::new();
    for item in iter {
        let item = item?;
        let trigger: PyRef<'_, PyTrigger> = item.extract().map_err(|_| {
            crate::error::AvisoError::new_err(
                "triggers= entries must be Trigger instances; got something else",
            )
        })?;
        out.push(trigger.clone().into_inner());
    }
    Ok(out)
}

pub(crate) fn register_clients(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyAvisoClient>()?;
    m.add_class::<PyAsyncAvisoClient>()?;
    Ok(())
}
