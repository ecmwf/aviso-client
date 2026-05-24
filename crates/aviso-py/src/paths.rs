//! Shared path normalisation helper.
//!
//! Every `PyO3` method that accepts a path declares its parameter as
//! `&Bound<'_, PyAny>` and runs it through `normalize_path` here. The helper
//! calls Python's `os.fspath` to accept any `os.PathLike`, then
//! `pathlib.Path(p).expanduser()` to expand `~`, then converts to a Rust
//! `PathBuf`. Doing the expansion once at the FFI boundary means every
//! path-typed argument across the binding receives identical handling.

use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::PyString;

/// Normalises a Python path-like value into a Rust `PathBuf`.
pub(crate) fn normalize_path(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    let os = py.import("os")?;
    let pathlib = py.import("pathlib")?;
    let fspath = os.call_method1("fspath", (value,))?;
    let path_cls = pathlib.getattr("Path")?;
    let path_obj = path_cls.call1((fspath,))?;
    let expanded = path_obj.call_method0("expanduser")?;
    let string_repr = expanded.str()?;
    let py_str: &Bound<'_, PyString> = string_repr.cast()?;
    Ok(PathBuf::from(py_str.to_cow()?.into_owned()))
}
