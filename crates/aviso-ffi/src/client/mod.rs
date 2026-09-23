// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Client and client-builder handles, and the blocking verbs over them.
//! Credential setters live in [`auth`].

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char};
use std::ptr;

use aviso::NotificationRequest;
use serde_json::Value;

use crate::error::{self, OutcomeError};
use crate::outcome::AvisoOutcome;
use crate::{guard, guard_outcome, reject_blocking_on_runtime, runtime};

mod auth;

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
    /// Kept so credential discovery can check where the credential would go.
    base_url: String,
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
/// duration of the call. The lifetime `'a` is not tied to anything the
/// compiler can see: it is whatever the call site infers. Use the result
/// within the entry point that received `ptr`, while the caller's string is
/// still live, and never store it in a handle that outlives the call.
pub(crate) unsafe fn cstr_opt<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null (checked above) and, per this function's #
    // Safety, points to a NUL-terminated C string valid for the call.
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Reads an optional string argument, distinguishing "not supplied" (null) from
/// "supplied but not UTF-8". A null pointer is a valid absent value (`Ok(None)`);
/// a non-UTF-8 pointer is an error the caller surfaces as `InvalidInput`.
///
/// # Safety
///
/// As [`cstr_opt`]: `ptr`, when non-null, must point to a NUL-terminated C
/// string valid for the duration of the call, and the borrow must not outlive
/// the entry point that received `ptr`.
pub(crate) unsafe fn cstr_nullable<'a>(ptr: *const c_char) -> Result<Option<&'a str>, ()> {
    if ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: `ptr` is non-null (checked above) and, per this function's #
    // Safety, points to a NUL-terminated C string valid for the call.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(Some)
        .map_err(|_| ())
}

/// Parses the `identifier_json` argument as a JSON object with arbitrary JSON
/// values.
pub(crate) fn parse_identifier(
    text: &str,
) -> Result<BTreeMap<String, serde_json::Value>, OutcomeError> {
    let value: Value = serde_json::from_str(text).map_err(|err| {
        error::invalid_input(&format!("identifier_json is not valid JSON: {err}"))
    })?;
    let Value::Object(object) = value else {
        return Err(error::invalid_input(
            "identifier_json must be a JSON object",
        ));
    };
    Ok(object.into_iter().collect())
}

/// Parses one element of the `notifications_json` array into a request.
///
/// An element is a JSON object with a required string `event_type`, an optional
/// `identifier` object with arbitrary JSON values, and an optional `payload`
/// of any shape. A `null` `identifier` or `payload`, like an absent one, is
/// omitted. Any shape violation is an `InvalidInput` error naming the index.
fn parse_notification_request(
    index: usize,
    value: Value,
) -> Result<NotificationRequest, OutcomeError> {
    let Value::Object(mut object) = value else {
        return Err(error::invalid_input(&format!(
            "notifications[{index}] must be a JSON object"
        )));
    };
    let event_type = match object.remove("event_type") {
        Some(Value::String(event_type)) => event_type,
        Some(_) => {
            return Err(error::invalid_input(&format!(
                "notifications[{index}].event_type must be a string"
            )));
        }
        None => {
            return Err(error::invalid_input(&format!(
                "notifications[{index}] is missing required key \"event_type\""
            )));
        }
    };
    let mut request = NotificationRequest::new(event_type);
    match object.remove("identifier") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            let identifier = map.into_iter().collect();
            request = request.with_identifier(identifier);
        }
        Some(_) => {
            return Err(error::invalid_input(&format!(
                "notifications[{index}].identifier must be a JSON object"
            )));
        }
    }
    match object.remove("payload") {
        None | Some(Value::Null) => {}
        Some(payload) => request = request.with_payload(payload),
    }
    Ok(request)
}

/// Parses the whole `notifications_json` array up front. Any malformed element
/// aborts the batch with an error and publishes nothing.
fn parse_notifications(text: &str) -> Result<Vec<NotificationRequest>, OutcomeError> {
    let value: Value = serde_json::from_str(text).map_err(|err| {
        error::invalid_input(&format!("notifications_json is not valid JSON: {err}"))
    })?;
    let Value::Array(items) = value else {
        return Err(error::invalid_input(
            "notifications_json must be a JSON array",
        ));
    };
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| parse_notification_request(index, item))
        .collect()
}

/// One element of the JSON array `aviso_client_notify_many` returns. `status` is
/// `"ok"` with `response` set, or `"error"` with `error` set.
#[derive(serde::Serialize)]
struct NotifyManyItem<'a> {
    index: usize,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<&'a aviso::NotifyResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<NotifyManyItemError>,
}

/// The structured error carried by a failed `NotifyManyItem`, mirroring the
/// fields of the C `AvisoError` struct.
#[derive(serde::Serialize)]
struct NotifyManyItemError {
    kind: &'static str,
    http_status: u16,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
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
            base_url: String::new(),
        };
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(base_url) } {
            Some(url) => {
                url.clone_into(&mut builder.base_url);
                builder.apply(|b| b.base_url(url));
            }
            None => {
                builder.error = Some(error::invalid_input(
                    "base_url must be non-null and valid UTF-8",
                ));
            }
        }
        Box::into_raw(Box::new(builder))
    })
}

/// Creates a client builder from the aviso config file, with a credential
/// found the way the `aviso` binary finds one.
///
/// Reads `~/.config/aviso/config.yaml`, or the file named in
/// `AVISO_CLIENT_CONFIG_FILE`, for the base URL, timeouts and TLS settings,
/// then searches the environment, the file's `auth:` block and the
/// credentials file for a credential. A missing file sets nothing; a file
/// that exists but cannot be used, or a credential that may not travel to the
/// configured address, is remembered and reported at build time. Setters
/// called afterwards replace what the file said.
///
/// Returns a builder handle, or null only if an internal panic is trapped.
#[unsafe(no_mangle)]
pub extern "C" fn aviso_client_builder_from_file() -> *mut AvisoClientBuilder {
    guard(ptr::null_mut(), || {
        Box::into_raw(Box::new(builder_from(
            aviso::AvisoClientBuilder::from_file(),
        )))
    })
}

/// Like `aviso_client_builder_from_file`, reading the file at `path`. The
/// path must exist; a missing file is reported at build time.
///
/// # Safety
///
/// `path` must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_from_file_at(
    path: *const c_char,
) -> *mut AvisoClientBuilder {
    guard(ptr::null_mut(), || {
        // SAFETY: the contract above requires `path` to be a NUL-terminated C
        // string; `cstr_opt` yields `None` for null or non-UTF-8.
        let Some(path) = (unsafe { cstr_opt(path) }) else {
            return Box::into_raw(Box::new(AvisoClientBuilder {
                inner: None,
                error: Some(error::invalid_input(
                    "from_file_at path must be non-null and valid UTF-8",
                )),
                base_url: String::new(),
            }));
        };
        Box::into_raw(Box::new(builder_from(
            aviso::AvisoClientBuilder::from_file_at(path),
        )))
    })
}

/// Sets or replaces the base URL. After `aviso_client_builder_from_file` this
/// overrides what the file said, or supplies an address the file lacked. A
/// null or non-UTF-8 argument is remembered and reported at build time.
///
/// # Safety
///
/// `builder` must be a live builder handle. `base_url`, when non-null, must be
/// a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_base_url(
    builder: *mut AvisoClientBuilder,
    base_url: *const c_char,
) {
    guard((), || {
        // SAFETY: the contract above requires `builder` to be a live handle;
        // `as_mut` yields `None` for null.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            return;
        };
        if builder.error.is_some() {
            return;
        }
        // SAFETY: the contract above requires `base_url`, when non-null, to be
        // a NUL-terminated C string.
        let Some(url) = (unsafe { cstr_opt(base_url) }) else {
            builder.error = Some(error::invalid_input(
                "base_url must be non-null and valid UTF-8",
            ));
            return;
        };
        url.clone_into(&mut builder.base_url);
        builder.apply(|b| b.base_url(url));
    });
}

/// Wraps a builder result as a handle, carrying a failure to build time.
fn builder_from(result: aviso::Result<aviso::AvisoClientBuilder>) -> AvisoClientBuilder {
    match result {
        Ok(inner) => AvisoClientBuilder {
            base_url: inner.configured_base_url().unwrap_or_default().to_owned(),
            inner: Some(inner),
            error: None,
        },
        Err(err) => AvisoClientBuilder {
            inner: None,
            error: Some(error::map_error(&err)),
            base_url: String::new(),
        },
    }
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
        // SAFETY: `builder` is non-null (checked above) and, per this
        // function's # Safety, points to a valid, aligned handle slot that
        // nothing else touches during this call.
        let slot = unsafe { &mut *builder };
        if slot.is_null() {
            return error::invalid_input("builder is null or already consumed").into_outcome();
        }
        // Take ownership and null the caller's pointer before doing any work.
        // SAFETY: the caller's slot holds a handle this library created and has
        // not freed, and ownership passes back here once; the slot is nulled
        // right after so a second call is a no-op, per this function's #
        // Safety.
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
    // SAFETY: the pointer came from this library's constructor and is freed at
    // most once, per this function's # Safety.
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
    // SAFETY: the pointer came from this library's constructor and is freed at
    // most once, per this function's # Safety.
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
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
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
/// whose values may have any JSON shape; `payload_json`, when non-null, is any
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
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        let Some(event_type) = (unsafe { cstr_opt(event_type) }) else {
            return error::invalid_input("event_type must be non-null and valid UTF-8")
                .into_outcome();
        };

        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
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
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
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

/// Publishes many notifications concurrently and returns one result per
/// request, in input order, as a compact-JSON array string success value
/// (retrieve it with `aviso_outcome_take_string`).
///
/// `notifications_json` is a JSON array; each element is an object with a
/// required string `event_type`, an optional `identifier` object of
/// arbitrary JSON values, and an optional `payload` of any shape. A malformed
/// array or element is an `AvisoErrorKind_InvalidInput` error and nothing is
/// published. `max_concurrency` caps in-flight requests; `0` selects a default.
///
/// On a valid array the call always succeeds at the ABI level: per-item
/// failures are reported in the array, not as a call error. Each element is
/// `{"index":N,"status":"ok","response":{...}}` or `{"index":N,"status":"error",
/// "error":{"kind":"...","http_status":N,"message":"...","request_id":"..."}}`.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// `AvisoErrorKind_InvalidUsage` error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `notifications_json`,
/// when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_notify_many(
    client: *const AvisoClient,
    notifications_json: *const c_char,
    max_concurrency: usize,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        let Some(notifications_json) = (unsafe { cstr_opt(notifications_json) }) else {
            return error::invalid_input("notifications_json must be non-null and valid UTF-8")
                .into_outcome();
        };
        let requests = match parse_notifications(notifications_json) {
            Ok(requests) => requests,
            Err(err) => return err.into_outcome(),
        };

        let results = runtime().block_on(client.inner.notify_many(&requests, max_concurrency));
        let items: Vec<NotifyManyItem> = results
            .iter()
            .enumerate()
            .map(|(index, result)| match result {
                Ok(response) => NotifyManyItem {
                    index,
                    status: "ok",
                    response: Some(response),
                    error: None,
                },
                Err(err) => {
                    let mapped = error::map_error(err);
                    NotifyManyItem {
                        index,
                        status: "error",
                        response: None,
                        error: Some(NotifyManyItemError {
                            kind: error::kind_label(mapped.kind()),
                            http_status: mapped.http_status(),
                            message: mapped.message_string(),
                            request_id: mapped.request_id_string(),
                        }),
                    }
                }
            })
            .collect();
        json_text_outcome(serde_json::to_string(&items))
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
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
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
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
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
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
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
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
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
#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "test code: every unsafe block here is a call into the crate's own C ABI from Rust, with the arguments constructed a few lines above"
)]
mod tests {
    use super::*;
    use crate::AvisoErrorKind;
    use crate::outcome::{
        aviso_outcome_error, aviso_outcome_free, aviso_outcome_is_ok, aviso_outcome_take_client,
        aviso_outcome_take_string, aviso_string_free,
    };
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
    fn parse_identifier_preserves_structured_spatial_values() {
        let identifier =
            parse_identifier(r#"{"point":[46,8],"point_cloud":[[46,8],[47,9]],"class":"od"}"#)
                .ok()
                .expect("valid identifier object");
        assert_eq!(identifier["point"], serde_json::json!([46, 8]));
        assert_eq!(
            identifier["point_cloud"],
            serde_json::json!([[46, 8], [47, 9]])
        );
        assert_eq!(identifier["class"], "od");
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

    #[test]
    fn notify_many_rejects_null_client() {
        let json = cstr("[]");
        let outcome = unsafe { aviso_client_notify_many(ptr::null(), json.as_ptr(), 0) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
    }

    #[test]
    fn notify_many_rejects_null_json() {
        let client = build_client();
        let outcome = unsafe { aviso_client_notify_many(client, ptr::null(), 0) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_many_rejects_non_array_json() {
        let client = build_client();
        let json = cstr(r#"{"event_type":"mars"}"#);
        let outcome = unsafe { aviso_client_notify_many(client, json.as_ptr(), 0) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_many_rejects_element_missing_event_type() {
        let client = build_client();
        let json = cstr(r#"[{"identifier":{"class":"od"}}]"#);
        let outcome = unsafe { aviso_client_notify_many(client, json.as_ptr(), 0) };
        assert_kind(outcome, AvisoErrorKind::InvalidInput);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn notify_many_reports_per_item_errors_against_unreachable() {
        let client = build_client();
        let json = cstr(r#"[{"event_type":"mars"},{"event_type":"mars"}]"#);
        let outcome = unsafe { aviso_client_notify_many(client, json.as_ptr(), 0) };
        assert!(!outcome.is_null());
        assert!(
            unsafe { aviso_outcome_is_ok(outcome) },
            "a valid array yields a success outcome even when every item fails"
        );
        let text = unsafe { aviso_outcome_take_string(outcome) };
        assert!(!text.is_null());
        let json_str = unsafe { CStr::from_ptr(text) }
            .to_str()
            .expect("utf8")
            .to_owned();
        unsafe { aviso_string_free(text) };
        unsafe { aviso_outcome_free(outcome) };
        unsafe { aviso_client_free(client) };

        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("json array");
        let array = parsed.as_array().expect("array");
        assert_eq!(array.len(), 2);
        for (index, item) in array.iter().enumerate() {
            assert_eq!(item["index"].as_u64(), Some(index as u64));
            assert_eq!(item["status"], "error");
            assert_eq!(item["error"]["kind"], "transport");
        }
    }

    #[test]
    fn parse_notifications_preserves_structured_identifier_values() {
        let requests = parse_notifications(
            r#"[{"event_type":"observations","identifier":{"point_cloud":[[46,8],[47,9]]}}]"#,
        )
        .ok()
        .expect("valid notifications");
        assert_eq!(
            requests[0].identifier["point_cloud"],
            serde_json::json!([[46, 8], [47, 9]])
        );
    }
}
