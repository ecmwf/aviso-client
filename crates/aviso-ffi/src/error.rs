//! The structured error surface of the C ABI.
//!
//! Every fallible call returns an [`crate::outcome::AvisoOutcome`] that either
//! carries a success value or owns an error. The error is exposed to C as the
//! [`AvisoError`] struct, whose `const char*` fields borrow from the owning
//! outcome and stay valid until the outcome is freed.

use std::ffi::{CString, c_char};
use std::ptr;

use aviso::ClientError;
use aviso::watch::TriggerKindLabel;

use crate::outcome::AvisoOutcome;

/// Discriminates an [`AvisoError`].
///
/// The first ten kinds mirror the core `ClientError`; the rest are conditions
/// the ABI itself reports: bad arguments, misuse, internal faults, a caught
/// Rust panic, and an unmapped (future) core variant. The discriminants are
/// fixed for ABI stability.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvisoErrorKind {
    /// Network-level failure (connect, TLS, DNS, partial read).
    Transport = 0,
    /// Server returned a non-success HTTP status.
    Http = 1,
    /// Authentication setup or credential resolution failed.
    Auth = 2,
    /// A response body could not be decoded.
    Decode = 3,
    /// A `CloudEvent` envelope carried a malformed event id.
    MalformedEvent = 4,
    /// A gap was detected in a watch stream.
    HistoryGap = 5,
    /// A fatal wire-protocol condition was observed on a watch stream.
    StreamProtocol = 6,
    /// Configuration-time error (invalid source, missing field).
    Config = 7,
    /// A persistent state-store operation failed.
    StateStore = 8,
    /// A required trigger failed after all retries.
    Trigger = 9,
    /// An argument was null, not UTF-8, or otherwise malformed.
    InvalidInput = 10,
    /// The call was used incorrectly (for example, a blocking call from a
    /// runtime or callback thread).
    InvalidUsage = 11,
    /// An internal invariant was violated.
    Internal = 12,
    /// A Rust panic was caught at the boundary.
    Panic = 13,
    /// An unmapped core error variant (the core enum grew).
    Unknown = 14,
}

/// C-visible error detail. The `const char*` fields borrow from the owning
/// [`crate::outcome::AvisoOutcome`] and are valid until it is freed. A pointer
/// is null when the field does not apply (`request_id` when unknown,
/// `trigger_kind` / `error_kind` unless `kind` is [`AvisoErrorKind::Trigger`]).
#[repr(C)]
pub struct AvisoError {
    /// The error kind.
    pub kind: AvisoErrorKind,
    /// HTTP status when `kind` is [`AvisoErrorKind::Http`], else `0`.
    pub http_status: u16,
    /// Redacted, human-readable message (never null).
    pub message: *const c_char,
    /// Server-supplied request id, or null when unknown.
    pub request_id: *const c_char,
    /// Trigger label when `kind` is [`AvisoErrorKind::Trigger`], else null.
    pub trigger_kind: *const c_char,
    /// Trigger inner-error label when applicable, else null.
    pub error_kind: *const c_char,
}

/// Owned backing for an [`AvisoError`]. The `CString` fields keep the heap
/// buffers that `view`'s pointers reference alive for the outcome's lifetime.
pub(crate) struct OutcomeError {
    message: CString,
    request_id: Option<CString>,
    trigger_kind: Option<CString>,
    error_kind: Option<CString>,
    view: AvisoError,
}

impl OutcomeError {
    pub(crate) fn build(
        kind: AvisoErrorKind,
        http_status: u16,
        message: String,
        request_id: Option<String>,
        trigger_kind: Option<&str>,
        error_kind: Option<String>,
    ) -> Self {
        let mut error = OutcomeError {
            message: cstring_lossy(message),
            request_id: request_id.map(cstring_lossy),
            trigger_kind: trigger_kind.map(|s| cstring_lossy(s.to_string())),
            error_kind: error_kind.map(cstring_lossy),
            view: AvisoError {
                kind,
                http_status,
                message: ptr::null(),
                request_id: ptr::null(),
                trigger_kind: ptr::null(),
                error_kind: ptr::null(),
            },
        };
        // The pointers reference the stable heap buffers owned by the CString
        // fields, not this struct, so they survive the moves this value makes on
        // its way into the owning outcome. The `view` field's own address only
        // matters once the outcome is boxed, where it is then stable.
        error.view.message = error.message.as_ptr();
        error.view.request_id = opt_ptr(error.request_id.as_ref());
        error.view.trigger_kind = opt_ptr(error.trigger_kind.as_ref());
        error.view.error_kind = opt_ptr(error.error_kind.as_ref());
        error
    }

    /// Pointer to the C-visible view, valid for the owner's lifetime.
    pub(crate) fn view(&self) -> *const AvisoError {
        &raw const self.view
    }

    /// Wraps this error in an owned outcome and hands it to C as a raw pointer.
    pub(crate) fn into_outcome(self) -> *mut AvisoOutcome {
        AvisoOutcome::error(self).into_raw()
    }
}

fn opt_ptr(s: Option<&CString>) -> *const c_char {
    s.map_or(ptr::null(), |c| c.as_ptr())
}

/// Builds a `CString`, dropping any interior NUL bytes so construction never
/// fails on an arbitrary message.
fn cstring_lossy(s: String) -> CString {
    let bytes: Vec<u8> = s.into_bytes().into_iter().filter(|b| *b != 0).collect();
    CString::new(bytes).unwrap_or_default()
}

/// Maps a core [`ClientError`] onto an owned [`OutcomeError`]. The wildcard arm
/// keeps the mapping total across the `#[non_exhaustive]` core enum.
pub(crate) fn map_error(err: &ClientError) -> OutcomeError {
    use AvisoErrorKind as K;
    match err {
        ClientError::Transport(inner) => {
            OutcomeError::build(K::Transport, 0, inner.to_string(), None, None, None)
        }
        ClientError::Http {
            status,
            body,
            request_id,
        } => OutcomeError::build(
            K::Http,
            *status,
            format!("http {status}: {body}"),
            request_id.clone(),
            None,
            None,
        ),
        ClientError::Auth(message) => {
            OutcomeError::build(K::Auth, 0, message.clone(), None, None, None)
        }
        ClientError::Decode(inner) => {
            OutcomeError::build(K::Decode, 0, inner.to_string(), None, None, None)
        }
        ClientError::MalformedEvent(detail) => {
            OutcomeError::build(K::MalformedEvent, 0, detail.clone(), None, None, None)
        }
        ClientError::HistoryGap { reason } => OutcomeError::build(
            K::HistoryGap,
            0,
            format!("history gap: {reason:?}"),
            None,
            None,
            None,
        ),
        ClientError::StreamProtocol {
            message,
            request_id,
        } => OutcomeError::build(
            K::StreamProtocol,
            0,
            message.clone(),
            request_id.clone(),
            None,
            None,
        ),
        ClientError::Config(message) => {
            OutcomeError::build(K::Config, 0, message.clone(), None, None, None)
        }
        ClientError::StateStore(inner) => {
            OutcomeError::build(K::StateStore, 0, inner.to_string(), None, None, None)
        }
        ClientError::TriggerFailed { kind, source: _ } => OutcomeError::build(
            K::Trigger,
            0,
            err.to_string(),
            None,
            Some(trigger_kind_label(kind)),
            None,
        ),
        _ => OutcomeError::build(K::Unknown, 0, err.to_string(), None, None, None),
    }
}

/// Stable label for a trigger kind. Wildcard arm covers the
/// `#[non_exhaustive]` core enum.
fn trigger_kind_label(label: &TriggerKindLabel) -> &'static str {
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

pub(crate) fn invalid_input(message: &str) -> OutcomeError {
    OutcomeError::build(
        AvisoErrorKind::InvalidInput,
        0,
        message.to_string(),
        None,
        None,
        None,
    )
}

pub(crate) fn invalid_usage(message: &str) -> OutcomeError {
    OutcomeError::build(
        AvisoErrorKind::InvalidUsage,
        0,
        message.to_string(),
        None,
        None,
        None,
    )
}

pub(crate) fn internal(message: &str) -> OutcomeError {
    OutcomeError::build(
        AvisoErrorKind::Internal,
        0,
        message.to_string(),
        None,
        None,
        None,
    )
}

/// The outcome handed back when [`std::panic::catch_unwind`] traps a panic at
/// the boundary.
pub(crate) fn panic_outcome() -> *mut AvisoOutcome {
    OutcomeError::build(
        AvisoErrorKind::Panic,
        0,
        "a Rust panic was caught at the FFI boundary".to_string(),
        None,
        None,
        None,
    )
    .into_outcome()
}
