// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The watch request builder: an opaque handle the C caller fills in with
//! setters and hands to `aviso_client_watch` or `aviso_watch_list_add`.

use std::collections::BTreeMap;
use std::ffi::c_char;
use std::ptr;

use aviso::watch::{ResumeStart, Trigger, WatchRequest};
use serde_json::Value;

use crate::client::{cstr_nullable, cstr_opt};
use crate::error::{self, OutcomeError};
use crate::guard;
use crate::triggers::AvisoTrigger;

/// Opaque builder for a watch request. Setters mutate it in place; the first
/// bad argument is remembered and surfaced through `on_end` when the watch
/// starts. Consumed by `aviso_client_watch` (which nulls the caller's pointer)
/// or freed with `aviso_watch_request_free`.
pub struct AvisoWatchRequest {
    pub(crate) spec: Option<RequestSpec>,
    pub(crate) error: Option<OutcomeError>,
}

pub(crate) struct RequestSpec {
    event_type: String,
    filter: Option<BTreeMap<String, Value>>,
    mode: Mode,
    triggers: Vec<Trigger>,
}

enum Mode {
    Watch,
    WatchFrom(ResumeStart),
    ReplayOnly(ResumeStart),
}

impl RequestSpec {
    pub(crate) fn into_request(self) -> WatchRequest {
        let mut request = match self.mode {
            Mode::Watch => WatchRequest::watch(self.event_type),
            Mode::WatchFrom(start) => WatchRequest::watch_from(self.event_type, start),
            Mode::ReplayOnly(start) => WatchRequest::replay_only(self.event_type, start),
        };
        if let Some(filter) = self.filter {
            request = request.with_filter(filter);
        }
        if !self.triggers.is_empty() {
            request = request.with_triggers(self.triggers);
        }
        request
    }
}

impl AvisoWatchRequest {
    fn with_spec(&mut self, f: impl FnOnce(&mut RequestSpec)) {
        if self.error.is_some() {
            return;
        }
        if let Some(spec) = self.spec.as_mut() {
            f(spec);
        }
    }
}

/// Parses the `filter_json` argument: a JSON object whose values are arbitrary
/// JSON (a scalar for an exact match, an object for a spatial or range rule),
/// matching the core filter shape `BTreeMap<String, serde_json::Value>`.
fn parse_filter(text: &str) -> Result<BTreeMap<String, Value>, OutcomeError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|err| error::invalid_input(&format!("filter_json is not valid JSON: {err}")))?;
    match value {
        Value::Object(object) => Ok(object.into_iter().collect()),
        _ => Err(error::invalid_input("filter_json must be a JSON object")),
    }
}

/// Creates a live-watch request for `event_type`. Returns a handle (null only
/// if an internal panic is trapped); a null or non-UTF-8 `event_type` is
/// remembered and surfaced when the watch starts. Free an abandoned request
/// with `aviso_watch_request_free`.
///
/// # Safety
///
/// `event_type`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_new(
    event_type: *const c_char,
) -> *mut AvisoWatchRequest {
    guard(ptr::null_mut(), || {
        let mut request = AvisoWatchRequest {
            spec: None,
            error: None,
        };
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(event_type) } {
            Some(event_type) => {
                request.spec = Some(RequestSpec {
                    event_type: event_type.to_string(),
                    filter: None,
                    mode: Mode::Watch,
                    triggers: Vec::new(),
                });
            }
            None => {
                request.error = Some(error::invalid_input(
                    "event_type must be non-null and valid UTF-8",
                ));
            }
        }
        Box::into_raw(Box::new(request))
    })
}

/// Sets the identifier filter from a JSON object string (a null argument clears
/// it). A non-UTF-8 or non-object argument is remembered and surfaced when the
/// watch starts.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`.
/// `filter_json`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_set_filter_json(
    request: *mut AvisoWatchRequest,
    filter_json: *const c_char,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if request.error.is_some() {
            return;
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_nullable(filter_json) } {
            Ok(None) => request.with_spec(|spec| spec.filter = None),
            Ok(Some(text)) => match parse_filter(text) {
                Ok(filter) => request.with_spec(|spec| spec.filter = Some(filter)),
                Err(err) => request.error = Some(err),
            },
            Err(()) => {
                request.error = Some(error::invalid_input("filter_json must be valid UTF-8"));
            }
        }
    });
}

/// Resumes a live watch after the given sequence (the watch replays everything
/// after `sequence`, then goes live).
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_watch_from_sequence(
    request: *mut AvisoWatchRequest,
    sequence: u64,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        if let Some(request) = unsafe { request.as_mut() } {
            request.with_spec(|spec| {
                spec.mode = Mode::WatchFrom(ResumeStart::AfterSequence(sequence));
            });
        }
    });
}

/// Resumes a live watch from the given date string (server-defined format),
/// replaying from that point and then going live.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`. `date`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_watch_from_date(
    request: *mut AvisoWatchRequest,
    date: *const c_char,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if request.error.is_some() {
            return;
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(date) } {
            Some(date) => request
                .with_spec(|spec| spec.mode = Mode::WatchFrom(ResumeStart::Date(date.to_string()))),
            None => {
                request.error = Some(error::invalid_input(
                    "date must be non-null and valid UTF-8",
                ));
            }
        }
    });
}

/// Switches the request to replay-only from the given sequence: it replays
/// everything after `sequence` and then ends, without going live.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_replay_from_sequence(
    request: *mut AvisoWatchRequest,
    sequence: u64,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        if let Some(request) = unsafe { request.as_mut() } {
            request.with_spec(|spec| {
                spec.mode = Mode::ReplayOnly(ResumeStart::AfterSequence(sequence));
            });
        }
    });
}

/// Switches the request to replay-only from the given date string: it replays
/// from that point and then ends, without going live.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`. `date`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_replay_from_date(
    request: *mut AvisoWatchRequest,
    date: *const c_char,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if request.error.is_some() {
            return;
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(date) } {
            Some(date) => request.with_spec(|spec| {
                spec.mode = Mode::ReplayOnly(ResumeStart::Date(date.to_string()));
            }),
            None => {
                request.error = Some(error::invalid_input(
                    "date must be non-null and valid UTF-8",
                ));
            }
        }
    });
}

/// Attaches a trigger to the request, consuming the trigger handle: on entry
/// the trigger is taken and the caller's pointer is nulled (so a later free is
/// a safe no-op). A trigger built from a bad argument is remembered on the
/// request and surfaced through the watch's `on_end` when it starts.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`. `trigger`
/// must point to a trigger-handle pointer; `*trigger`, when non-null, must be a
/// live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_add_trigger(
    request: *mut AvisoWatchRequest,
    trigger: *mut *mut AvisoTrigger,
) {
    guard((), || {
        if trigger.is_null() {
            return;
        }
        // SAFETY: `trigger` is non-null (checked above) and, per this
        // function's # Safety, points to a valid, aligned handle slot that
        // nothing else touches during this call.
        let slot = unsafe { &mut *trigger };
        if slot.is_null() {
            return;
        }
        // Take ownership and null the caller's pointer before any other work.
        // SAFETY: the caller's slot holds a handle this library created and has
        // not freed, and ownership passes back here once; the slot is nulled
        // right after so a second call is a no-op, per this function's #
        // Safety.
        let owned = unsafe { Box::from_raw(*slot) };
        *slot = ptr::null_mut();

        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if let Some(err) = owned.error {
            if request.error.is_none() {
                request.error = Some(err);
            }
            return;
        }
        if let Some(trigger) = owned.trigger {
            request.with_spec(|spec| spec.triggers.push(trigger));
        }
    });
}

/// Frees an abandoned watch request. Requests consumed by `aviso_client_watch`
/// are already freed; calling this on the nulled-out pointer is a safe no-op.
///
/// # Safety
///
/// `request`, when non-null, must be a live handle from
/// `aviso_watch_request_new` that was not consumed by a watch start.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_free(request: *mut AvisoWatchRequest) {
    if request.is_null() {
        return;
    }
    // SAFETY: the pointer came from this library's constructor and is freed at
    // most once, per this function's # Safety.
    drop(unsafe { Box::from_raw(request) });
}
