//! Python exception hierarchy and conversion from [`aviso::ClientError`].
//!
//! Every Rust error variant maps onto a Python exception class rooted at
//! [`AvisoError`]. Structured fields on the Rust variants surface as
//! attributes on the Python exception instance, so callers can do
//! ``except HttpError as e: print(e.status, e.request_id)`` without
//! reparsing message strings.
//!
//! Discriminator strings (the `reason` on `HistoryGapError`, the
//! `trigger_kind` / `error_kind` on `TriggerError`) come from hand-coded
//! match arms in [`map_client_error`], not from `{:?}` debug formatting,
//! so the string values are part of the documented contract and stable
//! across Rust compiler upgrades.
//!
//! The source enums (`ClientError`, `GapReason`, `TriggerKindLabel`,
//! `TriggerError`) are `#[non_exhaustive]`. A new variant added in the
//! core crate will NOT cause a compile failure here; the wildcard arm
//! converts it into the base `AvisoError` with an "unhandled variant"
//! message so the new variant is still caught by `except AvisoError`.
//! The binding maintainer keeps the table in sync as new variants land.

use aviso::ClientError;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(aviso._native, AvisoError, PyException);
create_exception!(aviso._native, TransportError, AvisoError);
create_exception!(aviso._native, HttpError, AvisoError);

/// Registers the exception classes on the `aviso._native` module.
///
/// Called once from the `#[pymodule]` entry point. Subsequent commits
/// add `AuthError`, `DecodeError`, `MalformedEventError`,
/// `HistoryGapError`, `StreamProtocolError`, `ConfigError`,
/// `StateStoreError`, and `TriggerError` here.
pub(crate) fn register_exceptions(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("AvisoError", py.get_type::<AvisoError>())?;
    m.add("TransportError", py.get_type::<TransportError>())?;
    m.add("HttpError", py.get_type::<HttpError>())?;
    Ok(())
}

/// Converts a Rust [`ClientError`] into the matching Python exception.
///
/// Currently handles [`ClientError::Transport`] and [`ClientError::Http`];
/// subsequent commits extend the table to cover every variant. The
/// wildcard arm catches new variants safely so the binding never panics
/// on an unknown error from the core crate.
pub(crate) fn map_client_error(py: Python<'_>, err: ClientError) -> PyErr {
    match err {
        ClientError::Transport(inner) => TransportError::new_err(inner.to_string()),
        ClientError::Http {
            status,
            body,
            request_id,
        } => http_error(py, status, &body, request_id.as_deref()),
        other => AvisoError::new_err(format!("unhandled variant: {other}")),
    }
}

/// Constructs an [`HttpError`] instance with `.status`, `.body`, and
/// `.request_id` attached as Python attributes.
///
/// `PyO3`'s `create_exception!` macro builds exception classes without a
/// custom `__init__`, so structured fields are set on the instance via
/// `setattr` after construction rather than via positional or keyword
/// arguments. This is the documented pattern in the `PyO3` book and
/// matches what `polars`, `pydantic-core`, and `opendal` do for their
/// structured exception types.
fn http_error(py: Python<'_>, status: u16, body: &str, request_id: Option<&str>) -> PyErr {
    let message = match request_id {
        Some(rid) => format!("http {status} (request_id={rid:?}): {body}"),
        None => format!("http {status}: {body}"),
    };
    let err = HttpError::new_err(message);
    if let Err(set_err) = attach_http_attributes(py, &err, status, body, request_id) {
        return set_err;
    }
    err
}

fn attach_http_attributes(
    py: Python<'_>,
    err: &PyErr,
    status: u16,
    body: &str,
    request_id: Option<&str>,
) -> PyResult<()> {
    let instance = err.value(py);
    instance.setattr("status", status)?;
    instance.setattr("body", body)?;
    instance.setattr("request_id", request_id.into_pyobject(py)?)?;
    Ok(())
}

/// Test-only helper exposed for cross-language round-trip tests in the
/// Python test suite. Constructs a synthetic [`ClientError::Http`] (or
/// another variant per `kind`) and runs it through [`map_client_error`]
/// so the test can inspect the raised exception's class and attributes.
///
/// Not part of the public Python surface. Registered on the module under
/// `_provoke_error` so tests can call it; production code never does.
#[pyfunction]
#[pyo3(name = "_provoke_error", signature = (kind, /, *, status = None, body = None, request_id = None))]
pub(crate) fn provoke_error(
    py: Python<'_>,
    kind: &str,
    status: Option<u16>,
    body: Option<String>,
    request_id: Option<String>,
) -> PyResult<()> {
    let err = synthesise_error(kind, status, body, request_id)?;
    Err(map_client_error(py, err))
}

fn synthesise_error(
    kind: &str,
    status: Option<u16>,
    body: Option<String>,
    request_id: Option<String>,
) -> PyResult<ClientError> {
    match kind {
        "http" => Ok(ClientError::Http {
            status: status.unwrap_or(500),
            body: body.unwrap_or_default(),
            request_id,
        }),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown error kind {other:?}; the test helper only knows kinds added \
             alongside their map_client_error arm"
        ))),
    }
}

pub(crate) fn register_provoke_error(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(provoke_error, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_http_status_to_http_error() {
        Python::attach(|py| {
            let err = ClientError::Http {
                status: 418,
                body: "I'm a teapot".to_string(),
                request_id: Some("req-1".to_string()),
            };
            let py_err = map_client_error(py, err);
            assert!(py_err.is_instance_of::<HttpError>(py));
        });
    }

    #[test]
    fn unhandled_variants_fall_back_to_avisoerror() {
        Python::attach(|py| {
            let err = ClientError::Config("missing base_url".to_string());
            let py_err = map_client_error(py, err);
            assert!(py_err.is_instance_of::<AvisoError>(py));
            assert!(
                !py_err.is_instance_of::<HttpError>(py),
                "config errors must NOT route to HttpError in this commit; subsequent \
                 commits add a ConfigError class with a dedicated mapping arm"
            );
        });
    }
}
