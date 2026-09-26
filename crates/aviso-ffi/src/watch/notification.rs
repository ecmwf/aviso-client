// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The per-notification view passed to `on_notification`, and its accessors.

use std::ffi::{CString, c_char};
use std::ptr;

use aviso::Notification;

use crate::error;

/// A read-only view of one notification, valid only for the `on_notification`
/// call it is passed to. The `identifier` and `payload` are pre-serialized to
/// compact JSON; the accessors return borrowed pointers into this view.
pub struct AvisoNotification {
    event_type: CString,
    identifier_json: CString,
    payload_json: CString,
    sequence: u64,
}

impl AvisoNotification {
    pub(crate) fn from_core(notification: &Notification) -> Self {
        let identifier_json =
            serde_json::to_string(&notification.identifier).unwrap_or_else(|_| "{}".to_string());
        let payload_json =
            serde_json::to_string(&notification.payload).unwrap_or_else(|_| "null".to_string());
        Self {
            event_type: error::cstring_lossy(notification.event_type.clone()),
            identifier_json: error::cstring_lossy(identifier_json),
            payload_json: error::cstring_lossy(payload_json),
            sequence: notification.sequence,
        }
    }
}

/// Returns the notification's event type. The pointer is valid only for the
/// duration of the `on_notification` call.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_event_type(
    notification: *const AvisoNotification,
) -> *const c_char {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.event_type.as_ptr(),
        None => ptr::null(),
    }
}

/// Returns the notification's per-stream sequence number.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call. A null pointer returns `0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_sequence(
    notification: *const AvisoNotification,
) -> u64 {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.sequence,
        None => 0,
    }
}

/// Returns the notification's identifier as a compact-JSON object string. The
/// pointer is valid only for the duration of the `on_notification` call.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_identifier_json(
    notification: *const AvisoNotification,
) -> *const c_char {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.identifier_json.as_ptr(),
        None => ptr::null(),
    }
}

/// Returns the notification's payload as a compact-JSON string (`null` when the
/// payload was absent). The pointer is valid only for the duration of the
/// `on_notification` call.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_payload_json(
    notification: *const AvisoNotification,
) -> *const c_char {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.payload_json.as_ptr(),
        None => ptr::null(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
mod tests {
    use std::ffi::CStr;

    use super::*;

    fn cstr(value: &str) -> CString {
        CString::new(value).expect("cstring")
    }

    #[test]
    fn identifier_json_keeps_structured_values() {
        let notification = AvisoNotification {
            event_type: cstr("observations"),
            identifier_json: cstr(r#"{"point_cloud":[[46.0,8.0],[47.0,9.0]]}"#),
            payload_json: cstr("null"),
            sequence: 7,
        };
        // SAFETY: the pointer is to a live view on this stack frame, and the
        // returned string borrows from it while it is still alive.
        let identifier =
            unsafe { CStr::from_ptr(aviso_notification_identifier_json(&raw const notification)) };
        let parsed: serde_json::Value =
            serde_json::from_str(identifier.to_str().expect("UTF-8")).expect("identifier JSON");
        assert_eq!(
            parsed["point_cloud"],
            serde_json::json!([[46.0, 8.0], [47.0, 9.0]])
        );
        // SAFETY: the pointer is to a live view on this stack frame.
        let sequence = unsafe { aviso_notification_sequence(&raw const notification) };
        assert_eq!(sequence, 7);
    }
}
