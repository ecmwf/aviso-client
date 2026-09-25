// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Sync and async notification iterators backing `AvisoClient.listen`
//! and `AsyncAvisoClient.listen`.
//!
//! The sync iterator wraps each `recv` in a 100ms `tokio::time::timeout`
//! and runs `py.check_signals()` between polls so Ctrl+C on an idle
//! stream raises `KeyboardInterrupt` within one poll period instead of
//! waiting indefinitely for the next notification.
//!
//! The async iterator uses `future_into_py` to expose the recv as a
//! Python awaitable; asyncio's standard signal handling delivers
//! `KeyboardInterrupt` to the awaiting task.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use aviso::watch::NotificationStream;
use futures_util::StreamExt;
use pyo3::exceptions::{PyStopAsyncIteration, PyStopIteration};
use pyo3::prelude::*;
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::error::map_client_error;
use crate::runtime::runtime;
use crate::values::PyNotification;

const SYNC_RECV_POLL: Duration = Duration::from_millis(100);

#[pyclass(
    name = "NotificationIterator",
    module = "pyaviso._native",
    skip_from_py_object
)]
pub(crate) struct PyNotificationIterator {
    inner: Arc<AsyncMutex<Option<NotificationStream>>>,
}

#[pymethods]
impl PyNotificationIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<PyNotification> {
        loop {
            let stream = self.inner.clone();
            let outcome: PollOutcome = py.detach(|| {
                runtime().block_on(async move {
                    let mut guard = stream.lock().await;
                    let Some(stream_ref) = guard.as_mut() else {
                        return PollOutcome::Closed;
                    };
                    match tokio::time::timeout(SYNC_RECV_POLL, stream_ref.next()).await {
                        Ok(Some(Ok(n))) => PollOutcome::Received(n),
                        Ok(Some(Err(e))) => PollOutcome::Error(e),
                        Ok(None) => PollOutcome::Exhausted,
                        Err(_) => PollOutcome::Timeout,
                    }
                })
            });
            py.check_signals()?;
            match outcome {
                PollOutcome::Received(n) => return Ok(PyNotification::from_core(n)),
                PollOutcome::Timeout => {}
                PollOutcome::Exhausted | PollOutcome::Closed => {
                    return Err(PyStopIteration::new_err(()));
                }
                PollOutcome::Error(e) => return Err(map_client_error(py, e)),
            }
        }
    }

    fn close(&self, py: Python<'_>) {
        let stream = self.inner.clone();
        py.detach(|| {
            runtime().block_on(async move {
                let mut guard = stream.lock().await;
                if let Some(s) = guard.take() {
                    s.close().await;
                }
            });
        });
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (exc_type = None, exc_value = None, traceback = None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        exc_type: Option<&Bound<'_, PyAny>>,
        exc_value: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        let _ = (exc_type, exc_value, traceback);
        self.close(py);
        false
    }
}

impl PyNotificationIterator {
    pub(crate) fn new(stream: NotificationStream) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(stream))),
        }
    }
}

enum PollOutcome {
    Received(aviso::Notification),
    Timeout,
    Exhausted,
    Closed,
    Error(aviso::ClientError),
}

#[pyclass(
    name = "AsyncNotificationIterator",
    module = "pyaviso._native",
    skip_from_py_object
)]
pub(crate) struct PyAsyncNotificationIterator {
    inner: Arc<AsyncMutex<Option<NotificationStream>>>,
    closed: Arc<AtomicBool>,
    wake: Arc<Notify>,
}

#[pymethods]
impl PyAsyncNotificationIterator {
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
            let Some(stream_ref) = guard.as_mut() else {
                return Err(PyStopAsyncIteration::new_err(()));
            };
            tokio::select! {
                biased;
                () = &mut woken => Err(PyStopAsyncIteration::new_err(())),
                item = stream_ref.next() => match item {
                    Some(Ok(n)) => Ok(PyNotification::from_core(n)),
                    Some(Err(e)) => Err(Python::attach(|py| map_client_error(py, e))),
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
            let mut guard = stream.lock().await;
            if let Some(s) = guard.take() {
                s.close().await;
            }
            Ok(())
        })
    }

    fn __aenter__<'py>(slf: PyRef<'_, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let slf_object: Py<PyAny> = slf.into_pyobject(py)?.into_any().unbind();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf_object) })
    }

    #[pyo3(signature = (exc_type = None, exc_value = None, traceback = None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        exc_type: Option<&Bound<'_, PyAny>>,
        exc_value: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let _ = (exc_type, exc_value, traceback);
        self.aclose(py)
    }
}

impl PyAsyncNotificationIterator {
    pub(crate) fn new(stream: NotificationStream) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(stream))),
            closed: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(Notify::new()),
        }
    }
}

pub(crate) fn register_streams(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyNotificationIterator>()?;
    m.add_class::<PyAsyncNotificationIterator>()?;
    Ok(())
}
