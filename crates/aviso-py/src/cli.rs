// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Bridges the bundled `aviso` console command into the extension.
//!
//! The `pyaviso` wheel declares an `aviso` console script (in
//! `pyproject.toml`) whose Python entry point calls [`run_cli`]. Running the
//! CLI in-process through the already-loaded extension is how one wheel ships
//! both `import pyaviso` and the `aviso` command, the layout maturin
//! recommends for shipping a library and a command together.

use pyo3::prelude::*;

/// Runs the `aviso` command-line client with `argv` and returns its exit code.
///
/// `argv` is the full process argument vector (`sys.argv`), program name
/// included. The GIL is released for the whole run so the CLI owns its async
/// runtime and signal handling. Private and unstable: it backs only the
/// bundled console script and is not part of the public `pyaviso` API.
#[pyfunction]
#[pyo3(name = "_run_cli")]
fn run_cli(py: Python<'_>, argv: Vec<String>) -> i32 {
    py.detach(|| aviso_cli::run(argv))
}

pub(crate) fn register_cli(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    Ok(())
}
