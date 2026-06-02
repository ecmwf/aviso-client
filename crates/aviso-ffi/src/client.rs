//! Client and client-builder handles, and the blocking verbs over them.

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char};
use std::ptr;
use std::sync::Arc;

use aviso::NotificationRequest;
use aviso::auth::Basic;
use serde_json::Value;

use crate::error::{self, OutcomeError};
use crate::outcome::AvisoOutcome;
use crate::{guard, guard_outcome, reject_blocking_on_runtime, runtime};

/// Opaque client handle. Wraps the core client (cheap to clone, shares the
/// connection pool and auth state).
pub struct AvisoClient {
    pub(crate) inner: aviso::AvisoClient,
}

/// Opaque client-builder handle. Setters mutate it in place; the first error
/// (a bad argument or an auth-construction failure) is remembered and surfaced
/// at `aviso_client_builder_build`.
pub struct AvisoClientBuilder {
    inner: Option<aviso::AvisoClientBuilder>,
    error: Option<OutcomeError>,
}

impl AvisoClientBuilder {
    fn apply(&mut self, f: impl FnOnce(aviso::AvisoClientBuilder) -> aviso::AvisoClientBuilder) {
        if let Some(inner) = self.inner.take() {
            self.inner = Some(f(inner));
        }
    }
}

/// Reads a borrowed string argument, or `None` when null or not UTF-8.
///
/// # Safety
///
/// `ptr`, when non-null, must point to a NUL-terminated C string valid for the
/// duration of the call.
pub(crate) unsafe fn cstr_opt<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Reads an optional string argument, distinguishing "not supplied" (null) from
/// "supplied but not UTF-8". A null pointer is a valid absent value (`Ok(None)`);
/// a non-UTF-8 pointer is an error the caller surfaces as `InvalidInput`.
///
/// # Safety
///
/// `ptr`, when non-null, must point to a NUL-terminated C string valid for the
/// duration of the call.
pub(crate) unsafe fn cstr_nullable<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(Some)
        .map_err(|_| ())
}

/// Parses the `identifier_json` argument: a JSON object whose values are all
/// strings, matching the core `BTreeMap<String, String>` identifier shape.
pub(crate) fn parse_identifier(text: &str) -> Result<BTreeMap<String, String>, OutcomeError> {
    let value: Value = serde_json::from_str(text).map_err(|err| {
        error::invalid_input(&format!("identifier_json is not valid JSON: {err}"))
    })?;
    let Value::Object(object) = value else {
        return Err(error::invalid_input(
            "identifier_json must be a JSON object of string to string",
        ));
    };
    let mut identifier = BTreeMap::new();
    for (key, value) in object {
        let Value::String(value) = value else {
            return Err(error::invalid_input(&format!(
                "identifier_json value for {key:?} must be a string"
            )));
        };
        identifier.insert(key, value);
    }
    Ok(identifier)
}

/// Wraps a serialized response as a string success outcome (see
/// [`AvisoOutcome::json_text`]) and hands it to C as a raw pointer.
fn json_text_outcome(json: serde_json::Result<String>) -> *mut AvisoOutcome {
    AvisoOutcome::json_text(json).into_raw()
}

/// Creates a client builder for `base_url`. Returns a builder handle (null only
/// if an internal panic is trapped); a null or non-UTF-8 `base_url` is
/// remembered and reported at build time. Free an abandoned builder with
/// `aviso_client_builder_free`.
///
/// # Safety
///
/// `base_url`, when non-null, must point to a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_new(
    base_url: *const c_char,
) -> *mut AvisoClientBuilder {
    guard(ptr::null_mut(), || {
        let mut builder = AvisoClientBuilder {
            inner: Some(aviso::AvisoClient::builder()),
            error: None,
        };
        match unsafe { cstr_opt(base_url) } {
            Some(url) => builder.apply(|b| b.base_url(url)),
            None => {
                builder.error = Some(error::invalid_input(
                    "base_url must be non-null and valid UTF-8",
                ));
            }
        }
        Box::into_raw(Box::new(builder))
    })
}

/// Sets HTTP Basic credentials on the builder. A null or non-UTF-8 argument, or
/// a credential-construction failure, is remembered and reported at build time.
///
/// # Safety
///
/// `builder` must be a live builder handle from `aviso_client_builder_new`.
/// `username` and `password`, when non-null, must be NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_basic_auth(
    builder: *mut AvisoClientBuilder,
    username: *const c_char,
    password: *const c_char,
) {
    guard((), || {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            return;
        };
        if builder.error.is_some() {
            return;
        }
        let (Some(user), Some(pass)) =
            (unsafe { cstr_opt(username) }, unsafe { cstr_opt(password) })
        else {
            builder.error = Some(error::invalid_input(
                "basic_auth username and password must be non-null and valid UTF-8",
            ));
            return;
        };
        match Basic::new(user, pass) {
            Ok(basic) => builder.apply(|b| b.auth(Arc::new(basic))),
            Err(err) => builder.error = Some(error::map_error(&err)),
        }
    });
}

/// Builds the client, consuming the builder. On entry the builder is taken and
/// the caller's pointer is set to null (so a later free is a safe no-op), even
/// when the build fails. Returns an outcome carrying the client (retrieve it
/// with `aviso_outcome_take_client`) or a structured error.
///
/// # Safety
///
/// `builder` must point to a builder-handle pointer. `*builder`, when non-null,
/// must be a live handle from `aviso_client_builder_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_build(
    builder: *mut *mut AvisoClientBuilder,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        if builder.is_null() {
            return error::invalid_input("builder handle pointer must not be null").into_outcome();
        }
        let slot = unsafe { &mut *builder };
        if slot.is_null() {
            return error::invalid_input("builder is null or already consumed").into_outcome();
        }
        // Take ownership and null the caller's pointer before doing any work.
        let owned = unsafe { Box::from_raw(*slot) };
        *slot = ptr::null_mut();

        if let Some(err) = owned.error {
            return err.into_outcome();
        }
        let Some(inner) = owned.inner else {
            return error::internal("builder was already consumed").into_outcome();
        };
        match inner.build() {
            Ok(client) => AvisoOutcome::client(Box::new(AvisoClient { inner: client })).into_raw(),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Frees an abandoned client builder. Builders consumed by
/// `aviso_client_builder_build` are already freed; calling this on the
/// nulled-out pointer is a safe no-op.
///
/// # Safety
///
/// `builder`, when non-null, must be a live handle from
/// `aviso_client_builder_new` that was not consumed by a build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_free(builder: *mut AvisoClientBuilder) {
    if builder.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(builder) });
}

/// Frees a client. A null pointer is a no-op.
///
/// # Safety
///
/// `client`, when non-null, must be a live handle and not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_free(client: *mut AvisoClient) {
    if client.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(client) });
}

/// Fetches the schema catalog (`GET /api/v1/schema`) and returns it as a
/// compact-JSON string success value (retrieve it with
/// `aviso_outcome_take_string`), or a structured error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_schema(client: *const AvisoClient) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        match runtime().block_on(client.inner.schema()) {
            Ok(catalog) => json_text_outcome(serde_json::to_string(&catalog)),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Publishes a notification (`POST /api/v1/notification`) and returns the
/// server's response as a compact-JSON string success value (retrieve it with
/// `aviso_outcome_take_string`), or a structured error.
///
/// `event_type` is required. `identifier_json`, when non-null, is a JSON object
/// of string-to-string identifier pairs; `payload_json`, when non-null, is any
/// JSON value. A null `identifier_json` or `payload_json` means the field is
/// omitted; a non-UTF-8 or otherwise malformed argument is an
/// `AvisoErrorKind_InvalidInput` error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `event_type`,
/// `identifier_json`, and `payload_json`, when non-null, must be NUL-terminated
/// C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_notify(
    client: *const AvisoClient,
    event_type: *const c_char,
    identifier_json: *const c_char,
    payload_json: *const c_char,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        let Some(event_type) = (unsafe { cstr_opt(event_type) }) else {
            return error::invalid_input("event_type must be non-null and valid UTF-8")
                .into_outcome();
        };

        let identifier = match unsafe { cstr_nullable(identifier_json) } {
            Ok(None) => BTreeMap::new(),
            Ok(Some(text)) => match parse_identifier(text) {
                Ok(identifier) => identifier,
                Err(err) => return err.into_outcome(),
            },
            Err(()) => {
                return error::invalid_input("identifier_json must be valid UTF-8").into_outcome();
            }
        };
        let payload = match unsafe { cstr_nullable(payload_json) } {
            Ok(None) => None,
            Ok(Some(text)) => match serde_json::from_str::<Value>(text) {
                Ok(value) => Some(value),
                Err(err) => {
                    return error::invalid_input(&format!("payload_json is not valid JSON: {err}"))
                        .into_outcome();
                }
            },
            Err(()) => {
                return error::invalid_input("payload_json must be valid UTF-8").into_outcome();
            }
        };

        let mut request = NotificationRequest::new(event_type).with_identifier(identifier);
        if let Some(payload) = payload {
            request = request.with_payload(payload);
        }

        match runtime().block_on(client.inner.notify(&request)) {
            Ok(response) => json_text_outcome(serde_json::to_string(&response)),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Fetches the schema for one event type (`GET /api/v1/schema/{event_type}`)
/// and returns it as a compact-JSON string success value (retrieve it with
/// `aviso_outcome_take_string`), or a structured error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `event_type`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_schema_for(
    client: *const AvisoClient,
    event_type: *const c_char,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        let Some(event_type) = (unsafe { cstr_opt(event_type) }) else {
            return error::invalid_input("event_type must be non-null and valid UTF-8")
                .into_outcome();
        };
        match runtime().block_on(client.inner.schema_for(event_type)) {
            Ok(response) => json_text_outcome(serde_json::to_string(&response)),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Wipes every notification for one stream (`DELETE
/// /api/v1/admin/wipe/stream`). Operator-only. Returns an empty success outcome
/// or a structured error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `stream_name`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_wipe_stream(
    client: *const AvisoClient,
    stream_name: *const c_char,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        let Some(stream_name) = (unsafe { cstr_opt(stream_name) }) else {
            return error::invalid_input("stream_name must be non-null and valid UTF-8")
                .into_outcome();
        };
        match runtime().block_on(client.inner.wipe_stream(stream_name)) {
            Ok(()) => AvisoOutcome::empty().into_raw(),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Wipes every stream (`DELETE /api/v1/admin/wipe/all`). Operator-only. Returns
/// an empty success outcome or a structured error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_wipe_all(client: *const AvisoClient) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        match runtime().block_on(client.inner.wipe_all()) {
            Ok(()) => AvisoOutcome::empty().into_raw(),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Deletes a single notification by its `<event_type>@<sequence>` id (`DELETE
/// /api/v1/admin/notification/{id}`). Operator-only. Returns an empty success
/// outcome or a structured error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `notification_id`,
/// when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_delete_notification(
    client: *const AvisoClient,
    notification_id: *const c_char,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        let Some(notification_id) = (unsafe { cstr_opt(notification_id) }) else {
            return error::invalid_input("notification_id must be non-null and valid UTF-8")
                .into_outcome();
        };
        match runtime().block_on(client.inner.delete_notification(notification_id)) {
            Ok(()) => AvisoOutcome::empty().into_raw(),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
mod tests {
    use super::*;
    use crate::AvisoErrorKind;
    use crate::outcome::{aviso_outcome_error, aviso_outcome_free, aviso_outcome_take_client};
    use std::ffi::CString;

    fn build_client() -> *mut AvisoClient {
        let base = CString::new("http://127.0.0.1:1").expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(base.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        unsafe { aviso_outcome_free(outcome) };
        assert!(
            !client.is_null(),
            "an unreachable URL still builds a client"
        );
        client
    }

    fn assert_kind(outcome: *mut AvisoOutcome, expected: AvisoErrorKind) {
        assert!(!outcome.is_null());
        let err = unsafe { aviso_outcome_error(outcome) };
        assert!(!err.is_null(), "expected an error outcome");
        assert_eq!(unsafe { (*err).kind }, expected);
        unsafe { aviso_outcome_free(outcome) };
    }

    fn cstr(value: &str) -> CString {
        CString::new(value).expect("cstring")
    }

    #[test]
    fn notify_rejects_malformed_identifier_json() {
        let client = build_client();
        let event = cstr("test_event");
        let identifier = cstr("{not json");
        let outcome = unsafe {
            aviso_client_notify(client, event.as_ptr(), identifier.as_ptr(), ptr::null())
        };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_rejects_non_string_identifier_value() {
        let client = build_client();
        let event = cstr("test_event");
        let identifier = cstr(r#"{"date": 1}"#);
        let outcome = unsafe {
            aviso_client_notify(client, event.as_ptr(), identifier.as_ptr(), ptr::null())
        };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_rejects_identifier_that_is_not_an_object() {
        let client = build_client();
        let event = cstr("test_event");
        let identifier = cstr(r#"["date"]"#);
        let outcome = unsafe {
            aviso_client_notify(client, event.as_ptr(), identifier.as_ptr(), ptr::null())
        };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_rejects_invalid_payload_json() {
        let client = build_client();
        let event = cstr("test_event");
        let payload = cstr("{not json");
        let outcome =
            unsafe { aviso_client_notify(client, event.as_ptr(), ptr::null(), payload.as_ptr()) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_rejects_null_event_type() {
        let client = build_client();
        let outcome = unsafe { aviso_client_notify(client, ptr::null(), ptr::null(), ptr::null()) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_rejects_null_client() {
        let event = cstr("test_event");
        let outcome =
            unsafe { aviso_client_notify(ptr::null(), event.as_ptr(), ptr::null(), ptr::null()) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
    }

    #[test]
    fn schema_for_rejects_null_event_type() {
        let client = build_client();
        let outcome = unsafe { aviso_client_schema_for(client, ptr::null()) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn wipe_stream_rejects_null_stream_name() {
        let client = build_client();
        let outcome = unsafe { aviso_client_wipe_stream(client, ptr::null()) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn delete_notification_rejects_null_id() {
        let client = build_client();
        let outcome = unsafe { aviso_client_delete_notification(client, ptr::null()) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }
}
