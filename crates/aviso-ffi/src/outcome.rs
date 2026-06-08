// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The owning result handle returned by every fallible C ABI call.
//!
//! An `AvisoOutcome` either carries a success value or an error, never both.
//! The consumer inspects it (`aviso_outcome_is_ok`, `aviso_outcome_error`),
//! optionally removes the success value with the matching typed `take`, then
//! frees it with `aviso_outcome_free`. Freeing an outcome whose success value
//! was never taken frees that value too, so a dropped outcome never leaks.

use std::ffi::{CString, c_char};
use std::ptr;

use crate::client::AvisoClient;
use crate::error::{AvisoError, OutcomeError};

/// Result of a fallible C ABI call. Opaque to C; always freed with
/// `aviso_outcome_free`.
pub struct AvisoOutcome {
    success: Success,
    error: Option<OutcomeError>,
}

enum Success {
    /// A call that succeeds with no value (for example an admin verb).
    Empty,
    /// An owned, NUL-terminated string (for example schema JSON).
    Text(CString),
    /// An owned client handle (from a successful builder build).
    Client(Box<AvisoClient>),
}

impl AvisoOutcome {
    pub(crate) fn empty() -> Self {
        Self {
            success: Success::Empty,
            error: None,
        }
    }

    pub(crate) fn text(value: CString) -> Self {
        Self {
            success: Success::Text(value),
            error: None,
        }
    }

    /// Wraps an already-serialized response as a string success outcome,
    /// mapping a serialization failure or an interior NUL to an `Internal`
    /// error. Shared by the blocking and async verbs that return JSON text.
    pub(crate) fn json_text(json: serde_json::Result<String>) -> Self {
        match json {
            Ok(json) => match CString::new(json) {
                Ok(text) => Self::text(text),
                Err(_) => Self::error(crate::error::internal(
                    "response JSON contained an interior NUL",
                )),
            },
            Err(err) => Self::error(crate::error::internal(&format!(
                "response serialization failed: {err}"
            ))),
        }
    }

    pub(crate) fn client(client: Box<AvisoClient>) -> Self {
        Self {
            success: Success::Client(client),
            error: None,
        }
    }

    pub(crate) fn error(error: OutcomeError) -> Self {
        Self {
            success: Success::Empty,
            error: Some(error),
        }
    }

    pub(crate) fn into_raw(self) -> *mut AvisoOutcome {
        Box::into_raw(Box::new(self))
    }
}

/// Returns `true` when the outcome carries no error.
///
/// # Safety
///
/// `outcome`, when non-null, must be a live pointer returned by this library
/// and not yet freed. A null pointer is tolerated and returns `false`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_outcome_is_ok(outcome: *const AvisoOutcome) -> bool {
    let Some(outcome) = (unsafe { outcome.as_ref() }) else {
        return false;
    };
    outcome.error.is_none()
}

/// Returns a borrowed pointer to the structured error, or null when the outcome
/// is a success. The pointer is valid until the outcome is freed.
///
/// # Safety
///
/// `outcome`, when non-null, must be a live pointer returned by this library
/// and not yet freed. A null pointer is tolerated and returns null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_outcome_error(outcome: *const AvisoOutcome) -> *const AvisoError {
    let Some(outcome) = (unsafe { outcome.as_ref() }) else {
        return ptr::null();
    };
    match &outcome.error {
        Some(error) => error.view(),
        None => ptr::null(),
    }
}

/// Removes and returns the outcome's string success value, transferring
/// ownership to the caller, who must free it with `aviso_string_free`.
/// Returns null when the outcome holds no string (an error, an empty success,
/// or a value already taken).
///
/// # Safety
///
/// `outcome`, when non-null, must be a live pointer returned by this library
/// and not yet freed. A null pointer is tolerated and returns null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_outcome_take_string(outcome: *mut AvisoOutcome) -> *mut c_char {
    let Some(outcome) = (unsafe { outcome.as_mut() }) else {
        return ptr::null_mut();
    };
    match std::mem::replace(&mut outcome.success, Success::Empty) {
        Success::Text(text) => text.into_raw(),
        other => {
            outcome.success = other;
            ptr::null_mut()
        }
    }
}

/// Removes and returns the outcome's client success value, transferring
/// ownership to the caller, who must free it with `aviso_client_free`. Returns
/// null when the outcome holds no client.
///
/// # Safety
///
/// `outcome`, when non-null, must be a live pointer returned by this library
/// and not yet freed. A null pointer is tolerated and returns null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_outcome_take_client(outcome: *mut AvisoOutcome) -> *mut AvisoClient {
    let Some(outcome) = (unsafe { outcome.as_mut() }) else {
        return ptr::null_mut();
    };
    match std::mem::replace(&mut outcome.success, Success::Empty) {
        Success::Client(client) => Box::into_raw(client),
        other => {
            outcome.success = other;
            ptr::null_mut()
        }
    }
}

/// Frees an outcome. Any success value not taken out first is freed with it.
/// A null pointer is a no-op.
///
/// # Safety
///
/// `outcome` must be a pointer returned by this library and not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_outcome_free(outcome: *mut AvisoOutcome) {
    if outcome.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(outcome) });
}

/// Frees a string returned by an `..._take_string` accessor. A null pointer is
/// a no-op.
///
/// # Safety
///
/// `text` must be a pointer returned by this library's take-string accessors
/// and not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_string_free(text: *mut c_char) {
    if text.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(text) });
}
