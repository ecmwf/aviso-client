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

use crate::auth::{extract_provider, is_anonymous};

use crate::error::{duration_from_seconds, map_client_error};
use crate::runtime::runtime;
use crate::state_stores::extract_store;
use crate::streams::{PyAsyncNotificationIterator, PyNotificationIterator};
use crate::triggers::PyTrigger;
use crate::values::{
    PyNotifyResponse, PyNotifyResult, PySchemaCatalog, PySchemaResponse, validate_identifier,
};
use crate::watch::{PyWatchRequest, parse_resume_start};

/// Applies the `auth` argument of a client constructor to a builder.
///
/// `None` means "look for a credential": the environment, then the config
/// file, then the credentials file. A credential found that way is attached
/// through `found_auth`, so build refuses a plain non-loopback http address
/// for it. `Anonymous()` means "send nothing". Anything else is used as
/// given, which is the caller choosing where it goes.
fn resolve_auth(
    py: Python<'_>,
    auth: Option<&Bound<'_, PyAny>>,
    builder: aviso::AvisoClientBuilder,
) -> PyResult<aviso::AvisoClientBuilder> {
    match auth {
        Some(obj) if is_anonymous(obj) => Ok(builder.anonymous()),
        Some(obj) => Ok(builder.auth(extract_provider(obj)?)),
        None => {
            let paths = aviso::auth::DiscoveryPaths::from_env();
            match aviso::auth::discover_with(&paths).map_err(|e| map_client_error(py, e))? {
                Some(found) => Ok(builder.found_auth(found)),
                None => Ok(builder),
            }
        }
    }
}

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

/// Builds from the config file, then applies the constructor keyword
/// arguments as overrides.
///
/// `None` for an argument means "keep what the file said". For `auth`, the
/// file's contribution is whatever the credential search found; passing a
/// provider replaces it and `Anonymous()` removes it. The two booleans are
/// `Option` here, unlike in the constructor, because `False` must mean "not
/// mentioned" rather than "turn it off".
#[allow(clippy::too_many_arguments)]
fn builder_from_file(
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
) -> PyResult<aviso::AvisoClientBuilder> {
    // The same convention as every other path this binding accepts:
    // os.fspath, then expanduser, so "~/aviso.yaml" means the home directory.
    let mut builder = match path {
        Some(p) => aviso::AvisoClientBuilder::from_file_at(crate::paths::normalize_path(py, p)?),
        None => aviso::AvisoClientBuilder::from_file(),
    }
    .map_err(|e| map_client_error(py, e))?;
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }
    if let Some(obj) = auth {
        builder = if is_anonymous(obj) {
            builder.anonymous()
        } else {
            builder.auth(extract_provider(obj)?)
        };
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
        builder = builder.heartbeat_interval(duration_from_seconds("heartbeat_interval", secs)?);
    }
    if let Some(v) = danger_accept_invalid_certs {
        builder = builder.danger_accept_invalid_certs(v);
    }
    if let Some(v) = flush_cursor_on_exit {
        builder = builder.flush_cursor_on_exit(v);
    }
    Ok(builder)
}

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
        builder = resolve_auth(py, auth, builder)?;
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
        let builder = builder_from_file(
            py,
            path,
            base_url,
            auth,
            timeout,
            user_agent,
            state_store,
            heartbeat_interval,
            danger_accept_invalid_certs,
            flush_cursor_on_exit,
        )?;
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
        builder = resolve_auth(py, auth, builder)?;
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
        let builder = builder_from_file(
            py,
            path,
            base_url,
            auth,
            timeout,
            user_agent,
            state_store,
            heartbeat_interval,
            danger_accept_invalid_certs,
            flush_cursor_on_exit,
        )?;
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

fn build_watch_request(
    event_type: Option<String>,
    filter: Option<&Bound<'_, PyDict>>,
    start_from: Option<&Bound<'_, PyAny>>,
    mode: Option<&str>,
    triggers: Option<&Bound<'_, PyAny>>,
    request: Option<PyRef<'_, PyWatchRequest>>,
) -> PyResult<aviso::watch::WatchRequest> {
    if let Some(req) = request {
        if event_type.is_some() || filter.is_some() || start_from.is_some() {
            return Err(crate::error::AvisoError::new_err(
                "request is mutually exclusive with event_type, filter, and start_from",
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
    let mut req = match (effective_mode, start_from) {
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
                "replay_only mode requires start_from=<int sequence or date string>",
            ));
        }
        _ => {
            return Err(crate::error::AvisoError::new_err(format!(
                "unknown WatchMode {effective_mode:?}; expected 'watch' or 'replay_only'"
            )));
        }
    };
    if let Some(filter_dict) = filter {
        validate_identifier(filter_dict.as_any())?;
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
