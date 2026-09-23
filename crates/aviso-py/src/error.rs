// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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

use std::time::Duration;

use aviso::ClientError;
use aviso::state::StoreError;
use aviso::watch::{
    GapReason, TemplateErrorKind, TriggerError as CoreTriggerError, TriggerKindLabel,
};
use pyo3::IntoPyObjectExt;
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(pyaviso._native, AvisoError, PyException);
create_exception!(pyaviso._native, TransportError, AvisoError);
create_exception!(pyaviso._native, HttpError, AvisoError);
create_exception!(pyaviso._native, AuthError, AvisoError);
create_exception!(pyaviso._native, DecodeError, AvisoError);
create_exception!(pyaviso._native, MalformedEventError, AvisoError);
create_exception!(pyaviso._native, HistoryGapError, AvisoError);
create_exception!(pyaviso._native, StreamProtocolError, AvisoError);
create_exception!(pyaviso._native, ConfigError, AvisoError);
create_exception!(pyaviso._native, StateStoreError, AvisoError);
create_exception!(pyaviso._native, TriggerError, AvisoError);

/// Registers the exception classes on the `pyaviso._native` module.
///
/// Called once from the `#[pymodule]` entry point.
pub(crate) fn register_exceptions(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("AvisoError", py.get_type::<AvisoError>())?;
    m.add("TransportError", py.get_type::<TransportError>())?;
    m.add("HttpError", py.get_type::<HttpError>())?;
    m.add("AuthError", py.get_type::<AuthError>())?;
    m.add("DecodeError", py.get_type::<DecodeError>())?;
    m.add("MalformedEventError", py.get_type::<MalformedEventError>())?;
    m.add("HistoryGapError", py.get_type::<HistoryGapError>())?;
    m.add("StreamProtocolError", py.get_type::<StreamProtocolError>())?;
    m.add("ConfigError", py.get_type::<ConfigError>())?;
    m.add("StateStoreError", py.get_type::<StateStoreError>())?;
    m.add("TriggerError", py.get_type::<TriggerError>())?;
    Ok(())
}

/// Converts a Rust [`ClientError`] into the matching Python exception.
///
/// The wildcard arm catches new variants safely so the binding never
/// panics on an unknown error from the core crate. Each documented
/// variant has its own arm with explicit field extraction; future
/// variants added in the core require a follow-up here.
pub(crate) fn map_client_error(py: Python<'_>, err: ClientError) -> PyErr {
    match err {
        ClientError::Transport(inner) => TransportError::new_err(inner.to_string()),
        ClientError::Http {
            status,
            body,
            request_id,
        } => http_error(py, status, &body, request_id.as_deref()),
        ClientError::Auth(message) => AuthError::new_err(message),
        ClientError::Decode(inner) => DecodeError::new_err(inner.to_string()),
        ClientError::MalformedEvent(detail) => MalformedEventError::new_err(detail),
        ClientError::HistoryGap { reason } => history_gap_error(py, reason),
        ClientError::StreamProtocol {
            message,
            request_id,
        } => stream_protocol_error(py, &message, request_id.as_deref()),
        ClientError::Config(message) => ConfigError::new_err(message),
        ClientError::StateStore(inner) => state_store_error(&inner),
        ClientError::TriggerFailed { kind, source } => trigger_error(py, &kind, source),
        other => AvisoError::new_err(format!("unhandled variant: {other}")),
    }
}

fn http_error(py: Python<'_>, status: u16, body: &str, request_id: Option<&str>) -> PyErr {
    let message = match request_id {
        Some(rid) => format!("http {status} (request_id={rid:?}): {body}"),
        None => format!("http {status}: {body}"),
    };
    let err = HttpError::new_err(message);
    match build_http_attrs(py, &err, status, body, request_id) {
        Ok(()) => err,
        Err(e) => e,
    }
}

fn build_http_attrs(
    py: Python<'_>,
    err: &PyErr,
    status: u16,
    body: &str,
    request_id: Option<&str>,
) -> PyResult<()> {
    let instance = err.value(py);
    instance.setattr("status", status)?;
    instance.setattr("body", body)?;
    instance.setattr("request_id", request_id.into_py_any(py)?)?;
    Ok(())
}

fn history_gap_error(py: Python<'_>, reason: GapReason) -> PyErr {
    let (reason_str, max_allowed, expected, observed) = match reason {
        GapReason::ReplayLimitReached { max_allowed } => {
            ("replay_limit_reached", Some(max_allowed), None, None)
        }
        GapReason::SequenceJump { expected, observed } => {
            ("sequence_jump", None, Some(expected), Some(observed))
        }
        _ => ("unknown", None, None, None),
    };
    let err = HistoryGapError::new_err(format!("history gap: {reason_str}"));
    match build_history_gap_attrs(py, &err, reason_str, max_allowed, expected, observed) {
        Ok(()) => err,
        Err(e) => e,
    }
}

fn build_history_gap_attrs(
    py: Python<'_>,
    err: &PyErr,
    reason: &str,
    max_allowed: Option<u64>,
    expected: Option<u64>,
    observed: Option<u64>,
) -> PyResult<()> {
    let instance = err.value(py);
    instance.setattr("reason", reason)?;
    instance.setattr("max_allowed", max_allowed.into_py_any(py)?)?;
    instance.setattr("expected", expected.into_py_any(py)?)?;
    instance.setattr("observed", observed.into_py_any(py)?)?;
    Ok(())
}

fn stream_protocol_error(py: Python<'_>, message: &str, request_id: Option<&str>) -> PyErr {
    let rendered = match request_id {
        Some(rid) => format!("stream protocol (request_id={rid:?}): {message}"),
        None => format!("stream protocol: {message}"),
    };
    let err = StreamProtocolError::new_err(rendered);
    match build_stream_protocol_attrs(py, &err, message, request_id) {
        Ok(()) => err,
        Err(e) => e,
    }
}

fn build_stream_protocol_attrs(
    py: Python<'_>,
    err: &PyErr,
    message: &str,
    request_id: Option<&str>,
) -> PyResult<()> {
    let instance = err.value(py);
    instance.setattr("message", message)?;
    instance.setattr("request_id", request_id.into_py_any(py)?)?;
    Ok(())
}

fn state_store_error(inner: &StoreError) -> PyErr {
    StateStoreError::new_err(inner.to_string())
}

fn trigger_error(py: Python<'_>, kind: &TriggerKindLabel, source: CoreTriggerError) -> PyErr {
    let trigger_kind = trigger_kind_label_str(kind);
    let log_path = match kind {
        TriggerKindLabel::Log { path } => Some(path.display().to_string()),
        _ => None,
    };
    let detail = TriggerErrorDetail::from_source(source);
    let rendered = format!(
        "trigger {trigger_kind} failed ({}): {}",
        detail.error_kind, detail.summary
    );
    let err = TriggerError::new_err(rendered);
    match build_trigger_attrs(py, &err, trigger_kind, log_path.as_deref(), &detail) {
        Ok(()) => err,
        Err(e) => e,
    }
}

fn build_trigger_attrs(
    py: Python<'_>,
    err: &PyErr,
    trigger_kind: &'static str,
    path: Option<&str>,
    detail: &TriggerErrorDetail,
) -> PyResult<()> {
    let instance = err.value(py);
    instance.setattr("trigger_kind", trigger_kind)?;
    instance.setattr("error_kind", detail.error_kind)?;
    instance.setattr("path", path.into_py_any(py)?)?;
    instance.setattr("exit_code", detail.exit_code.into_py_any(py)?)?;
    instance.setattr(
        "stderr_tail",
        detail.stderr_tail.as_deref().into_py_any(py)?,
    )?;
    instance.setattr("status", detail.status.into_py_any(py)?)?;
    instance.setattr("body_tail", detail.body_tail.as_deref().into_py_any(py)?)?;
    instance.setattr("reason", detail.reason.as_deref().into_py_any(py)?)?;
    instance.setattr("timeout_seconds", detail.timeout_seconds.into_py_any(py)?)?;
    instance.setattr("context", detail.context.as_deref().into_py_any(py)?)?;
    instance.setattr("field", detail.field.as_deref().into_py_any(py)?)?;
    instance.setattr("template_kind", detail.template_kind.into_py_any(py)?)?;
    Ok(())
}

fn trigger_kind_label_str(label: &TriggerKindLabel) -> &'static str {
    match label {
        TriggerKindLabel::Echo => "echo",
        TriggerKindLabel::Log { .. } => "log",
        #[cfg(unix)]
        TriggerKindLabel::Command => "command",
        TriggerKindLabel::Webhook => "webhook",
        TriggerKindLabel::Teams => "teams",
        TriggerKindLabel::Post => "post",
        _ => "unknown",
    }
}

#[derive(Default)]
struct TriggerErrorDetail {
    error_kind: &'static str,
    summary: String,
    exit_code: Option<i32>,
    stderr_tail: Option<String>,
    status: Option<u16>,
    body_tail: Option<String>,
    reason: Option<String>,
    timeout_seconds: Option<f64>,
    context: Option<String>,
    field: Option<String>,
    template_kind: Option<&'static str>,
}

impl TriggerErrorDetail {
    fn from_source(source: CoreTriggerError) -> Self {
        match source {
            CoreTriggerError::Io(err) => Self {
                error_kind: "io",
                summary: err.to_string(),
                ..Self::default()
            },
            CoreTriggerError::Encode(err) => Self {
                error_kind: "encode",
                summary: err.to_string(),
                ..Self::default()
            },
            #[cfg(unix)]
            CoreTriggerError::Command {
                exit_code,
                stderr_tail,
            } => Self {
                error_kind: "command",
                summary: format!("exit {exit_code}: {stderr_tail}"),
                exit_code: Some(exit_code),
                stderr_tail: Some(stderr_tail),
                ..Self::default()
            },
            CoreTriggerError::Timeout(d) => Self {
                error_kind: "timeout",
                summary: format!("trigger timed out after {d:?}"),
                timeout_seconds: Some(d.as_secs_f64()),
                ..Self::default()
            },
            CoreTriggerError::Webhook { status, body_tail } => Self {
                error_kind: "webhook",
                summary: format!(
                    "status={} body_tail={}",
                    status.map_or_else(|| "<transport>".to_string(), |s| s.as_u16().to_string(),),
                    if body_tail.is_empty() {
                        "<empty>"
                    } else {
                        body_tail.as_str()
                    },
                ),
                status: status.map(|s| s.as_u16()),
                body_tail: Some(body_tail),
                ..Self::default()
            },
            CoreTriggerError::WebhookBuild { reason } => Self {
                error_kind: "webhook_build",
                summary: reason.clone(),
                reason: Some(reason),
                ..Self::default()
            },
            CoreTriggerError::Template {
                context,
                field,
                kind,
            } => Self {
                error_kind: "template",
                summary: format!("render in {context} failed at {field}: {kind:?}"),
                context: Some(context),
                field: Some(field),
                template_kind: Some(template_kind_str(kind)),
                ..Self::default()
            },
            _ => Self {
                error_kind: "unknown",
                summary: "unhandled trigger error variant".into(),
                ..Self::default()
            },
        }
    }
}

fn template_kind_str(kind: TemplateErrorKind) -> &'static str {
    match kind {
        TemplateErrorKind::Missing => "missing",
        TemplateErrorKind::EnvNotSet => "env_not_set",
        TemplateErrorKind::EnvNotUnicode => "env_not_unicode",
        TemplateErrorKind::BadSyntax => "bad_syntax",
        TemplateErrorKind::ValueInUrlAuthority => "value_in_url_authority",
        TemplateErrorKind::ValueAfterUnsupportedShellSyntax => {
            "value_after_unsupported_shell_syntax"
        }
        TemplateErrorKind::NotificationEncode => "notification_encode",
        _ => "unknown",
    }
}

/// Converts a user-provided seconds-as-float into a `Duration`, raising
/// `ConfigError` on negative, NaN, or infinite values rather than letting
/// `Duration::from_secs_f64` panic across the FFI boundary.
pub(crate) fn duration_from_seconds(field: &str, seconds: f64) -> PyResult<Duration> {
    if !seconds.is_finite() {
        return Err(ConfigError::new_err(format!(
            "{field} must be a finite non-negative number of seconds; got {seconds}"
        )));
    }
    if seconds < 0.0 {
        return Err(ConfigError::new_err(format!(
            "{field} must be non-negative; got {seconds}"
        )));
    }
    Duration::try_from_secs_f64(seconds).map_err(|err| {
        ConfigError::new_err(format!(
            "{field} = {seconds} cannot be represented as a duration: {err}"
        ))
    })
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
    fn maps_config_to_config_error() {
        Python::attach(|py| {
            let err = ClientError::Config("missing base_url".to_string());
            let py_err = map_client_error(py, err);
            assert!(py_err.is_instance_of::<ConfigError>(py));
        });
    }

    #[test]
    fn maps_history_gap_replay_limit() {
        Python::attach(|py| {
            let err = ClientError::HistoryGap {
                reason: GapReason::ReplayLimitReached { max_allowed: 1000 },
            };
            let py_err = map_client_error(py, err);
            assert!(py_err.is_instance_of::<HistoryGapError>(py));
        });
    }

    #[test]
    fn maps_trigger_failed_echo() {
        Python::attach(|py| {
            let err = ClientError::TriggerFailed {
                kind: TriggerKindLabel::Echo,
                source: CoreTriggerError::Io(std::io::Error::other("broken pipe")),
            };
            let py_err = map_client_error(py, err);
            assert!(py_err.is_instance_of::<TriggerError>(py));
        });
    }
}
