// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `PyO3` wrapper around `aviso::watch::Trigger` and its six kind
//! constructors, plus `Trigger.function`, which calls a Python function.
//!
//! The six built-in kinds run inside the library, before a notification
//! reaches Python. A function trigger cannot: it must run in the caller's
//! thread (or on its event loop) so the function needs no locking and can be
//! a coroutine. So it never reaches the core; the iterator that delivers the
//! notification calls it, through `pyaviso._functions`.

use std::sync::Arc;

use aviso::watch::{HttpMethod, Trigger};
use pyo3::prelude::*;
use pyo3::types::PyList;

use crate::error::duration_from_seconds;
use crate::paths::normalize_path;

#[pyclass(name = "Trigger", module = "pyaviso._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTrigger {
    kind: TriggerKind,
}

#[derive(Clone)]
enum TriggerKind {
    Native(Trigger),
    Function(FunctionTrigger),
}

/// A Python function to call with each notification.
#[derive(Clone)]
pub(crate) struct FunctionTrigger {
    func: Arc<Py<PyAny>>,
    retries: u32,
    required: bool,
    label: Option<String>,
}

impl FunctionTrigger {
    /// Whether the function is `async def` (also through `functools.partial`).
    pub(crate) fn is_async(&self, py: Python<'_>) -> PyResult<bool> {
        is_coroutine_function(self.func.bind(py))
    }

    /// The name used in messages: the label, the function's qualified name,
    /// or, for callables without one such as `functools.partial`, its repr.
    pub(crate) fn name(&self, py: Python<'_>) -> PyResult<String> {
        if let Some(label) = &self.label {
            return Ok(label.clone());
        }
        let func = self.func.bind(py);
        if func.hasattr("__qualname__")? {
            return func.getattr("__qualname__")?.extract();
        }
        Ok(func.repr()?.to_string())
    }

    /// The shape `pyaviso._many` expects: `(func, retries, required, label)`.
    pub(crate) fn to_py<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        (
            self.func.clone_ref(py),
            self.retries,
            self.required,
            self.label.clone(),
        )
            .into_pyobject(py)
            .map(pyo3::Bound::into_any)
    }
}

impl PyTrigger {
    fn native(t: Trigger) -> Self {
        Self {
            kind: TriggerKind::Native(t),
        }
    }

    /// Applies a builder step to a built-in trigger, or refuses it for a
    /// function trigger with a message naming the setting.
    fn map_native(
        &self,
        setting: &str,
        f: impl FnOnce(Trigger) -> PyResult<Trigger>,
    ) -> PyResult<Self> {
        match &self.kind {
            TriggerKind::Native(t) => Ok(Self::native(f(t.clone())?)),
            TriggerKind::Function(_) => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "{setting} does not apply to Trigger.function: Python code cannot be \
                 interrupted or run in parallel safely"
            ))),
        }
    }
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
        Self::native(t)
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
        Ok(Self::native(t))
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
        Ok(Self::native(t))
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
        Ok(Self::native(t))
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
        Ok(Self::native(t))
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
        Ok(Self::native(t))
    }

    /// Calls `func` with each notification, in the thread (or on the event
    /// loop) that reads the notification, after the built-in triggers.
    #[staticmethod]
    #[pyo3(signature = (func, *, retries = 0, required = true, label = None))]
    fn function(
        func: &Bound<'_, PyAny>,
        retries: u32,
        required: bool,
        label: Option<String>,
    ) -> PyResult<Self> {
        if !func.is_callable() {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Trigger.function needs a callable that takes the notification",
            ));
        }
        Ok(Self {
            kind: TriggerKind::Function(FunctionTrigger {
                func: Arc::new(func.clone().unbind()),
                retries,
                required,
                label,
            }),
        })
    }

    fn retries(&self, n: u32) -> Self {
        let mut next = self.clone();
        match &mut next.kind {
            TriggerKind::Native(t) => *t = t.clone().retries(n),
            TriggerKind::Function(f) => f.retries = n,
        }
        next
    }

    fn required(&self, on: bool) -> Self {
        let mut next = self.clone();
        match &mut next.kind {
            TriggerKind::Native(t) => *t = t.clone().required(on),
            TriggerKind::Function(f) => f.required = on,
        }
        next
    }

    fn timeout(&self, seconds: f64) -> PyResult<Self> {
        let d = duration_from_seconds("timeout", seconds)?;
        self.map_native("timeout", |t| Ok(t.timeout(d)))
    }

    fn fail_fast(&self, on: bool) -> PyResult<Self> {
        self.map_native("fail_fast", |t| Ok(t.fail_fast(on)))
    }

    fn label(&self, name: String) -> Self {
        let mut next = self.clone();
        match &mut next.kind {
            TriggerKind::Native(t) => *t = t.clone().label(name),
            TriggerKind::Function(f) => f.label = Some(name),
        }
        next
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(match &self.kind {
            TriggerKind::Native(t) => format!("Trigger({t:?})"),
            TriggerKind::Function(f) => format!(
                "Trigger(function={}, retries={}, required={})",
                f.name(py)?,
                f.retries,
                if f.required { "True" } else { "False" }
            ),
        })
    }
}

/// `inspect.iscoroutinefunction`, which sees through `functools.partial`.
pub(crate) fn is_coroutine_function(func: &Bound<'_, PyAny>) -> PyResult<bool> {
    func.py()
        .import("inspect")?
        .call_method1("iscoroutinefunction", (func,))?
        .is_truthy()
}

/// Refuses `async def` functions where nothing would await them.
pub(crate) fn refuse_async_functions(
    py: Python<'_>,
    functions: &[FunctionTrigger],
    listener: Option<&str>,
) -> PyResult<()> {
    for f in functions {
        if f.is_async(py)? {
            let whose = listener.map_or_else(String::new, |n| format!("listener '{n}': "));
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "{whose}Trigger.function({}) is an async function; it needs \
                 AsyncAvisoClient, which awaits it. With AvisoClient, pass a plain function.",
                f.name(py)?
            )));
        }
    }
    Ok(())
}

/// Built-in triggers for the core, and function triggers for the iterator.
#[derive(Clone, Default)]
pub(crate) struct SplitTriggers {
    pub(crate) native: Vec<Trigger>,
    pub(crate) functions: Vec<FunctionTrigger>,
}

impl SplitTriggers {
    pub(crate) fn push(&mut self, trigger: &PyTrigger) {
        match &trigger.kind {
            TriggerKind::Native(t) => self.native.push(t.clone()),
            TriggerKind::Function(f) => self.functions.push(f.clone()),
        }
    }
}

/// Function triggers in the shape `pyaviso._functions` takes them.
pub(crate) fn function_list<'py>(
    py: Python<'py>,
    functions: &[FunctionTrigger],
) -> PyResult<Bound<'py, PyList>> {
    let items = functions
        .iter()
        .map(|f| f.to_py(py))
        .collect::<PyResult<Vec<_>>>()?;
    PyList::new(py, items)
}

/// `listen` with function triggers: wraps the native iterator so each
/// notification's functions are called as it is read.
pub(crate) fn wrap_listen(
    py: Python<'_>,
    iterator: Py<PyAny>,
    functions: &[FunctionTrigger],
    asynchronous: bool,
) -> PyResult<Py<PyAny>> {
    let class = if asynchronous {
        "AsyncFunctionTriggerIterator"
    } else {
        "FunctionTriggerIterator"
    };
    Ok(py
        .import("pyaviso._functions")?
        .getattr(class)?
        .call1((iterator, function_list(py, functions)?))?
        .unbind())
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
