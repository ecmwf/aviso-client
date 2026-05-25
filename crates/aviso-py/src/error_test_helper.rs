//! Test-only `_provoke_error` helper.
//!
//! Split out of [`super::error`] to keep that file under the AGENTS.md
//! 500-LOC split threshold. The helper constructs a synthetic
//! [`aviso::ClientError`] of the requested `kind`, runs it through
//! [`super::error::map_client_error`], and raises the resulting Python
//! exception. The Python test suite calls this through the registered
//! `aviso._native._provoke_error` symbol so each `ClientError` variant
//! round-trips through the mapper.
//!
//! Not part of the public Python surface. Production code never calls
//! this.

use aviso::ClientError;
use aviso::state::StoreError;
use aviso::watch::{
    GapReason, TemplateErrorKind, TriggerError as CoreTriggerError, TriggerKindLabel,
};
use pyo3::prelude::*;

use crate::error::map_client_error;

#[pyfunction]
#[pyo3(
    name = "_provoke_error",
    signature = (
        kind, /, *,
        status = None,
        body = None,
        request_id = None,
        message = None,
        detail = None,
        sequence = None,
        max_allowed = None,
        expected = None,
        observed = None,
        log_path = None,
    )
)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn provoke_error(
    py: Python<'_>,
    kind: &str,
    status: Option<u16>,
    body: Option<String>,
    request_id: Option<String>,
    message: Option<String>,
    detail: Option<String>,
    sequence: Option<u64>,
    max_allowed: Option<u64>,
    expected: Option<u64>,
    observed: Option<u64>,
    log_path: Option<String>,
) -> PyResult<()> {
    let err = synthesise_error(
        kind,
        status,
        body,
        request_id,
        message,
        detail,
        sequence,
        max_allowed,
        expected,
        observed,
        log_path,
    )?;
    Err(map_client_error(py, err))
}

#[allow(clippy::too_many_arguments)]
fn synthesise_error(
    kind: &str,
    status: Option<u16>,
    body: Option<String>,
    request_id: Option<String>,
    message: Option<String>,
    detail: Option<String>,
    _sequence: Option<u64>,
    max_allowed: Option<u64>,
    expected: Option<u64>,
    observed: Option<u64>,
    log_path: Option<String>,
) -> PyResult<ClientError> {
    match kind {
        "http" => Ok(ClientError::Http {
            status: status.unwrap_or(500),
            body: body.unwrap_or_default(),
            request_id,
        }),
        "auth" => Ok(ClientError::Auth(message.unwrap_or_default())),
        "decode" => {
            let attempt: Result<serde_json::Value, serde_json::Error> =
                serde_json::from_str("not json {");
            match attempt {
                Ok(_) => Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "could not synthesise a decode error: bogus json parsed successfully",
                )),
                Err(e) => Ok(ClientError::Decode(e)),
            }
        }
        "malformed_event" => Ok(ClientError::MalformedEvent(detail.unwrap_or_default())),
        "history_gap_replay_limit" => Ok(ClientError::HistoryGap {
            reason: GapReason::ReplayLimitReached {
                max_allowed: max_allowed.unwrap_or(0),
            },
        }),
        "history_gap_sequence_jump" => Ok(ClientError::HistoryGap {
            reason: GapReason::SequenceJump {
                expected: expected.unwrap_or(0),
                observed: observed.unwrap_or(0),
            },
        }),
        "stream_protocol" => Ok(ClientError::StreamProtocol {
            message: message.unwrap_or_default(),
            request_id,
        }),
        "config" => Ok(ClientError::Config(message.unwrap_or_default())),
        "state_store_io" => Ok(ClientError::StateStore(StoreError::Io(
            std::io::Error::other(message.unwrap_or_else(|| "disk".to_string())),
        ))),
        "trigger_failed_echo_io" => Ok(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Echo,
            source: CoreTriggerError::Io(std::io::Error::other(
                message.unwrap_or_else(|| "broken pipe".to_string()),
            )),
        }),
        "trigger_failed_log_io" => Ok(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Log {
                path: std::path::PathBuf::from(log_path.unwrap_or_else(|| "/tmp/log".into())),
            },
            source: CoreTriggerError::Io(std::io::Error::other(
                message.unwrap_or_else(|| "permission denied".to_string()),
            )),
        }),
        "trigger_failed_webhook_4xx" => {
            let parsed_status = match status {
                Some(code) => Some(reqwest::StatusCode::from_u16(code).map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err(format!(
                        "status {code} is not a valid HTTP status code"
                    ))
                })?),
                None => None,
            };
            Ok(ClientError::TriggerFailed {
                kind: TriggerKindLabel::Webhook,
                source: CoreTriggerError::Webhook {
                    status: parsed_status,
                    body_tail: body.unwrap_or_default(),
                },
            })
        }
        "trigger_failed_template_missing" => Ok(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Webhook,
            source: CoreTriggerError::Template {
                context: detail.unwrap_or_else(|| "webhook url".to_string()),
                field: message.unwrap_or_else(|| "notification.payload.target".to_string()),
                kind: TemplateErrorKind::Missing,
            },
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
