// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The native half of `listen_many`, and of `listen` with function triggers.
//!
//! This side opens the watches through `AvisoClient::watch_many` and hands
//! out raw items: `(name, Notification)`, or `(name, exception)` when a watch
//! fails, with the exception's `listener` attribute set to the name. What to
//! do with them (call function triggers, apply `on_error`, `run()`) lives in
//! `pyaviso._many`, in Python, because it calls back into the caller's code:
//! functions that may be coroutines, and an `on_error` that may raise.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use aviso::watch::{ErrorPolicy, MultiNotificationStream};
use futures_util::StreamExt;
use pyo3::exceptions::{PyStopAsyncIteration, PyStopIteration, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::error::map_client_error;
use crate::requests::{RequestSpec, build_watch_request};
use crate::runtime::runtime;
use crate::values::PyNotification;
use crate::watch::{PyWatchRequest, parse_resume_start};

const SYNC_RECV_POLL: Duration = Duration::from_millis(100);

/// Keys an entry dict may have; anything else is a typo and is refused.
const ENTRY_KEYS: [&str; 5] = ["event_type", "filter", "start_from", "mode", "triggers"];

type Shared = Arc<AsyncMutex<Option<MultiNotificationStream>>>;

/// One merged item as Python sees it.
fn to_python(
    py: Python<'_>,
    item: Result<(String, aviso::Notification), aviso::watch::EntryError>,
) -> PyResult<Py<PyAny>> {
    match item {
        Ok((name, n)) => (name, PyNotification::from_core(n))
            .into_pyobject(py)
            .map(|t| t.into_any().unbind()),
        Err(e) => {
            let error = map_client_error(py, e.error).into_value(py);
            error.bind(py).setattr("listener", &e.name)?;
            (e.name, error)
                .into_pyobject(py)
                .map(|t| t.into_any().unbind())
        }
    }
}

enum Poll {
    Item(Result<(String, aviso::Notification), aviso::watch::EntryError>),
    Timeout,
    Ended,
}

/// Synchronous raw iterator over a merged stream.
#[pyclass(
    name = "_RawMultiIterator",
    module = "pyaviso._native",
    skip_from_py_object
)]
pub(crate) struct PyRawMultiIterator {
    inner: Shared,
}

#[pymethods]
impl PyRawMultiIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        loop {
            let stream = self.inner.clone();
            let outcome = py.detach(|| {
                runtime().block_on(async move {
                    let mut guard = stream.lock().await;
                    let Some(s) = guard.as_mut() else {
                        return Poll::Ended;
                    };
                    match tokio::time::timeout(SYNC_RECV_POLL, s.next()).await {
                        Ok(Some(item)) => Poll::Item(item),
                        Ok(None) => Poll::Ended,
                        Err(_) => Poll::Timeout,
                    }
                })
            });
            // Ctrl+C is noticed between polls, as for `listen`.
            py.check_signals()?;
            match outcome {
                Poll::Item(item) => return to_python(py, item),
                Poll::Timeout => {}
                Poll::Ended => return Err(PyStopIteration::new_err(())),
            }
        }
    }

    fn close(&self, py: Python<'_>) {
        let stream = self.inner.clone();
        py.detach(|| {
            runtime().block_on(async move {
                if let Some(s) = stream.lock().await.take() {
                    s.close().await;
                }
            });
        });
    }
}

/// Asynchronous raw iterator over a merged stream.
#[pyclass(
    name = "_RawAsyncMultiIterator",
    module = "pyaviso._native",
    skip_from_py_object
)]
pub(crate) struct PyRawAsyncMultiIterator {
    inner: Shared,
    closed: Arc<AtomicBool>,
    wake: Arc<Notify>,
}

#[pymethods]
impl PyRawAsyncMultiIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = self.inner.clone();
        let closed = self.closed.clone();
        let wake = self.wake.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            if closed.load(Ordering::Acquire) {
                return Err(PyStopAsyncIteration::new_err(()));
            }
            let mut guard = stream.lock().await;
            // Register for aclose()'s wake-up before checking the flag, so
            // a close between the two cannot be missed.
            let woken = wake.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if closed.load(Ordering::Acquire) {
                return Err(PyStopAsyncIteration::new_err(()));
            }
            let Some(s) = guard.as_mut() else {
                return Err(PyStopAsyncIteration::new_err(()));
            };
            tokio::select! {
                biased;
                () = &mut woken => Err(PyStopAsyncIteration::new_err(())),
                item = s.next() => match item {
                    Some(item) => Python::attach(|py| to_python(py, item)),
                    None => Err(PyStopAsyncIteration::new_err(())),
                },
            }
        })
    }

    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = self.inner.clone();
        let closed = self.closed.clone();
        let wake = self.wake.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            closed.store(true, Ordering::Release);
            wake.notify_waiters();
            if let Some(s) = stream.lock().await.take() {
                s.close().await;
            }
            Ok(())
        })
    }
}

/// The caller's `on_error` choice.
enum OnError<'py> {
    Raise,
    Continue,
    Function(Bound<'py, PyAny>),
}

impl<'py> OnError<'py> {
    /// Reads `on_error`; leaving it out means "raise".
    fn parse(on_error: Option<&Bound<'py, PyAny>>) -> PyResult<Self> {
        let Some(value) = on_error else {
            return Ok(Self::Raise);
        };
        if let Ok(text) = value.cast::<PyString>() {
            return match text.to_str()? {
                "raise" => Ok(Self::Raise),
                "continue" => Ok(Self::Continue),
                other => Err(PyValueError::new_err(format!(
                    "on_error must be 'raise', 'continue' or a function, not '{other}'"
                ))),
            };
        }
        if value.is_callable() {
            return Ok(Self::Function(value.clone()));
        }
        Err(PyTypeError::new_err(
            "on_error must be 'raise', 'continue' or a function taking (name, error)",
        ))
    }

    /// What `pyaviso._many` takes: the string, or the function.
    fn to_python(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        match self {
            Self::Raise => PyString::new(py, "raise").into_any(),
            Self::Continue => PyString::new(py, "continue").into_any(),
            Self::Function(f) => f.clone(),
        }
    }
}

/// Puts the listener's name in front of the error's message and on its
/// `listener` attribute, keeping the exception itself (type, attributes,
/// cause) as it was. If that cannot be done, the failure is raised instead,
/// with the original error as its cause, rather than being dropped.
fn named(py: Python<'_>, name: &str, err: PyErr) -> PyErr {
    match label(py, name, &err) {
        Ok(()) => err,
        Err(failure) => {
            failure.set_cause(py, Some(err));
            failure
        }
    }
}

fn label(py: Python<'_>, name: &str, err: &PyErr) -> PyResult<()> {
    let value = err.value(py);
    let args = value.getattr("args")?.cast_into::<PyTuple>()?;
    // Only a plain message is prefixed; an exception whose first argument is
    // not a string keeps its message and still gets the attribute.
    if !args.is_empty()
        && let Ok(message) = args.get_item(0)?.cast::<PyString>()
    {
        let prefixed = format!("listener '{name}': {}", message.to_str()?);
        let mut items: Vec<Bound<'_, PyAny>> = Vec::with_capacity(args.len());
        items.push(PyString::new(py, &prefixed).into_any());
        items.extend(args.iter().skip(1));
        value.setattr("args", PyTuple::new(py, items)?)?;
    }
    value.setattr("listener", name)
}

/// A `dict` view of any mapping; `None` when `obj` is not one.
fn as_dict<'py>(obj: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyDict>>> {
    if let Ok(d) = obj.cast::<PyDict>() {
        return Ok(Some(d.clone()));
    }
    let py = obj.py();
    let mapping = py.import("collections.abc")?.getattr("Mapping")?;
    if obj.is_instance(&mapping)? {
        let d = py.import("builtins")?.getattr("dict")?.call1((obj,))?;
        return Ok(Some(d.cast_into::<PyDict>()?));
    }
    Ok(None)
}

/// One entry of `listen_many`: a dict of `listen()` keywords, or a
/// `WatchRequest`.
fn entry_spec(
    py: Python<'_>,
    name: &str,
    entry: &Bound<'_, PyAny>,
    start_from: Option<&Bound<'_, PyAny>>,
    mode: Option<&str>,
) -> PyResult<RequestSpec> {
    if let Ok(request) = entry.extract::<PyRef<'_, PyWatchRequest>>() {
        return Ok(request.clone().into_spec());
    }
    let Some(dict) = as_dict(entry)? else {
        return Err(PyTypeError::new_err(format!(
            "listener '{name}' must be a dict of listen() arguments or a WatchRequest"
        )));
    };
    for key in dict.keys() {
        let key: String = key.extract().map_err(|_| {
            PyTypeError::new_err(format!("listener '{name}': keys must be strings"))
        })?;
        if !ENTRY_KEYS.contains(&key.as_str()) {
            return Err(PyValueError::new_err(format!(
                "listener '{name}': unknown key '{key}'; expected one of {}",
                ENTRY_KEYS.join(", ")
            )));
        }
    }
    let event_type: String = dict
        .get_item("event_type")?
        .ok_or_else(|| PyValueError::new_err(format!("listener '{name}' needs an event_type")))?
        .extract()
        .map_err(|_| {
            PyTypeError::new_err(format!("listener '{name}': event_type must be a str"))
        })?;
    let filter = match dict.get_item("filter")? {
        Some(f) if !f.is_none() => Some(f.cast_into::<PyDict>().map_err(|_| {
            PyTypeError::new_err(format!("listener '{name}': filter must be a dict"))
        })?),
        _ => None,
    };
    let own_start = dict.get_item("start_from")?.filter(|v| !v.is_none());
    // A `WatchMode` member is a str subclass, so extracting it yields its
    // value ("replay_only"); `str()` of it would give "WatchMode.REPLAY_ONLY".
    let own_mode: Option<String> = match dict.get_item("mode")? {
        Some(m) if !m.is_none() => Some(m.extract().map_err(|_| {
            PyTypeError::new_err(format!(
                "listener '{name}': mode must be a str or WatchMode"
            ))
        })?),
        _ => None,
    };
    let triggers = dict.get_item("triggers")?.filter(|v| !v.is_none());
    let start = own_start.as_ref().or(start_from);
    let entry_mode = own_mode.as_deref().or(mode);
    build_watch_request(
        Some(event_type),
        filter.as_ref(),
        start,
        entry_mode,
        triggers.as_ref(),
        None,
    )
    .map_err(|e| named(py, name, e))
}

/// `listen_many` for both clients: opens the watches and returns the
/// `pyaviso._many` iterator that delivers them.
pub(crate) fn listen_many(
    py: Python<'_>,
    client: &aviso::AvisoClient,
    listeners: &Bound<'_, PyAny>,
    start_from: Option<&Bound<'_, PyAny>>,
    mode: Option<&str>,
    on_error: Option<&Bound<'_, PyAny>>,
    asynchronous: bool,
) -> PyResult<Py<PyAny>> {
    let on_error = OnError::parse(on_error)?;
    if let OnError::Function(f) = &on_error
        && !asynchronous
        && crate::triggers::is_coroutine_function(f)?
    {
        return Err(PyTypeError::new_err(
            "on_error is an async function; it needs AsyncAvisoClient, which awaits it",
        ));
    }
    let Some(listeners) = as_dict(listeners)? else {
        return Err(PyTypeError::new_err(
            "listen_many takes a dict mapping a name to each listener",
        ));
    };
    if listeners.is_empty() {
        return Err(PyValueError::new_err(
            "listen_many needs at least one listener",
        ));
    }
    // The shared options are checked even when every entry sets its own, so
    // a mistake in them is reported rather than silently unused.
    if let Some(m) = mode
        && m != "watch"
        && m != "replay_only"
    {
        return Err(PyValueError::new_err(format!(
            "mode must be 'watch' or 'replay_only', not '{m}'"
        )));
    }
    if let Some(value) = start_from {
        parse_resume_start(value)?;
    }
    let mut requests = Vec::with_capacity(listeners.len());
    let functions = PyDict::new(py);
    for (key, entry) in listeners.iter() {
        let name: String = key
            .extract()
            .map_err(|_| PyTypeError::new_err("listener names must be strings"))?;
        if name.is_empty() {
            return Err(PyValueError::new_err("listener names must not be empty"));
        }
        let spec = entry_spec(py, &name, &entry, start_from, mode)?;
        // The core checks each request again before opening it; checking
        // here first names the listener the way every other argument error
        // does.
        client
            .check_watch_request(&spec.request)
            .map_err(|e| named(py, &name, map_client_error(py, e)))?;
        if !asynchronous {
            crate::triggers::refuse_async_functions(py, &spec.functions, Some(&name))?;
        }
        functions.set_item(&name, crate::triggers::function_list(py, &spec.functions)?)?;
        requests.push((name, spec.request));
    }
    let client = client.clone();
    let stream = py
        .detach(|| {
            runtime().block_on(async move { client.watch_many(requests, ErrorPolicy::Continue) })
        })
        .map_err(|e| map_client_error(py, e))?;
    let shared: Shared = Arc::new(AsyncMutex::new(Some(stream)));
    let module = py.import("pyaviso._many")?;
    let (raw, class): (Py<PyAny>, &str) = if asynchronous {
        let raw = PyRawAsyncMultiIterator {
            inner: shared,
            closed: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(Notify::new()),
        };
        (
            Py::new(py, raw)?.into_any(),
            "AsyncMultiNotificationIterator",
        )
    } else {
        (
            Py::new(py, PyRawMultiIterator { inner: shared })?.into_any(),
            "MultiNotificationIterator",
        )
    };
    Ok(module
        .getattr(class)?
        .call1((raw, functions, on_error.to_python(py), listeners.len()))?
        .unbind())
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyRawMultiIterator>()?;
    m.add_class::<PyRawAsyncMultiIterator>()?;
    Ok(())
}
