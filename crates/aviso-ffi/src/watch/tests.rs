// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The single-watch C ABI: requests, notification views and the handle.

#![allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "test code: every unsafe block here is a call into the crate's own C ABI from Rust, with the arguments constructed a few lines above"
)]

use std::ffi::{CString, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use super::handle::{aviso_client_watch, aviso_watch_free, aviso_watch_stop, aviso_watch_wait};
use super::notification::AvisoNotification;
use super::request::{
    aviso_watch_request_add_trigger, aviso_watch_request_free, aviso_watch_request_new,
};
use crate::AvisoErrorKind;
use crate::client::{
    AvisoClient, aviso_client_builder_build, aviso_client_builder_new, aviso_client_free,
};
use crate::outcome::{
    AvisoOutcome, aviso_outcome_error, aviso_outcome_free, aviso_outcome_take_client,
};

/// How a test's `on_end` saw the watch finish.
struct Sink {
    ended: AtomicBool,
    end_kind: AtomicI32,
}

impl Sink {
    fn new() -> Self {
        Self {
            ended: AtomicBool::new(false),
            end_kind: AtomicI32::new(-2),
        }
    }
}

/// None of these tests reaches a server, so no notification arrives.
extern "C" fn on_notification(_ctx: *mut c_void, _notification: *const AvisoNotification) -> bool {
    true
}

extern "C" fn on_end(ctx: *mut c_void, outcome: *mut AvisoOutcome) {
    let sink = unsafe { &*(ctx as *const Sink) };
    let error = unsafe { aviso_outcome_error(outcome) };
    let kind = if error.is_null() {
        -1
    } else {
        unsafe { (*error).kind as i32 }
    };
    sink.end_kind.store(kind, Ordering::SeqCst);
    sink.ended.store(true, Ordering::SeqCst);
    unsafe { aviso_outcome_free(outcome) };
}

fn build_client() -> *mut AvisoClient {
    let base = CString::new("http://127.0.0.1:1").expect("cstring");
    let mut builder = unsafe { aviso_client_builder_new(base.as_ptr()) };
    let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
    let client = unsafe { aviso_outcome_take_client(outcome) };
    unsafe { aviso_outcome_free(outcome) };
    assert!(!client.is_null());
    client
}

fn cstr(value: &str) -> CString {
    CString::new(value).expect("cstring")
}

#[test]
fn watch_request_new_and_free_roundtrip() {
    let event = cstr("test_event");
    let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
    assert!(!request.is_null());
    unsafe { aviso_watch_request_free(request) };

    // A null event type still yields a freeable handle (the error is
    // remembered for the watch start).
    let bad = unsafe { aviso_watch_request_new(ptr::null()) };
    assert!(!bad.is_null());
    unsafe { aviso_watch_request_free(bad) };
}

#[test]
fn client_watch_null_request_pointer_returns_null() {
    let watch =
        unsafe { aviso_client_watch(ptr::null(), ptr::null_mut(), None, None, ptr::null_mut()) };
    assert!(watch.is_null());
}

#[test]
fn client_watch_consumes_request_even_when_rejected() {
    // Null callbacks: the start is rejected, but the request must still be
    // taken and the caller's pointer nulled so a later free is a no-op.
    let event = cstr("test_event");
    let mut request = unsafe { aviso_watch_request_new(event.as_ptr()) };
    let watch =
        unsafe { aviso_client_watch(ptr::null(), &raw mut request, None, None, ptr::null_mut()) };
    assert!(watch.is_null());
    assert!(request.is_null(), "the request pointer must be nulled");
}

#[test]
fn watch_starts_then_stops_gracefully() {
    // No server: the supervisor keeps trying to connect to an unreachable
    // URL. A stop request must break the loop, close the stream, and fire
    // on_end exactly once with a graceful (no-error) outcome.
    let client = build_client();
    let event = cstr("test_event");
    let mut request = unsafe { aviso_watch_request_new(event.as_ptr()) };

    let sink = Box::new(Sink::new());
    let ctx = (&raw const *sink) as *mut c_void;

    let watch = unsafe {
        aviso_client_watch(
            client,
            &raw mut request,
            Some(on_notification),
            Some(on_end),
            ctx,
        )
    };
    assert!(!watch.is_null());
    assert!(request.is_null(), "the request pointer must be nulled");

    unsafe { aviso_watch_stop(watch) };
    let outcome = unsafe { aviso_watch_wait(watch) };
    assert!(!outcome.is_null());
    assert!(unsafe { aviso_outcome_error(outcome) }.is_null());
    unsafe { aviso_outcome_free(outcome) };

    assert!(sink.ended.load(Ordering::SeqCst), "on_end must have fired");
    assert_eq!(
        sink.end_kind.load(Ordering::SeqCst),
        -1,
        "a graceful stop must carry no error"
    );

    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn add_trigger_consumes_and_nulls_the_handle() {
    let event = cstr("test_event");
    let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
    let mut trigger = crate::triggers::aviso_trigger_echo();
    unsafe { aviso_watch_request_add_trigger(request, &raw mut trigger) };
    assert!(trigger.is_null(), "the trigger pointer must be nulled");
    unsafe { aviso_watch_request_free(request) };
}

#[test]
fn watch_with_a_bad_trigger_reports_invalid_input_via_on_end() {
    // A trigger built from a bad argument (a null log path) is remembered on
    // the request and surfaced through on_end when the watch starts, before
    // any stream work.
    let client = build_client();
    let event = cstr("test_event");
    let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
    let mut trigger = unsafe { crate::triggers::aviso_trigger_log(ptr::null()) };
    unsafe { aviso_watch_request_add_trigger(request, &raw mut trigger) };

    let sink = Box::new(Sink::new());
    let ctx = (&raw const *sink) as *mut c_void;
    let mut request = request;
    let watch = unsafe {
        aviso_client_watch(
            client,
            &raw mut request,
            Some(on_notification),
            Some(on_end),
            ctx,
        )
    };
    assert!(!watch.is_null());
    let outcome = unsafe { aviso_watch_wait(watch) };
    unsafe { aviso_outcome_free(outcome) };
    assert!(sink.ended.load(Ordering::SeqCst));
    assert_eq!(
        sink.end_kind.load(Ordering::SeqCst),
        AvisoErrorKind::InvalidInput as i32
    );
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn watch_wait_is_idempotent_after_completion() {
    let client = build_client();
    let event = cstr("test_event");
    let mut request = unsafe { aviso_watch_request_new(event.as_ptr()) };
    let sink = Box::new(Sink::new());
    let ctx = (&raw const *sink) as *mut c_void;
    let watch = unsafe {
        aviso_client_watch(
            client,
            &raw mut request,
            Some(on_notification),
            Some(on_end),
            ctx,
        )
    };
    unsafe { aviso_watch_stop(watch) };
    let first = unsafe { aviso_watch_wait(watch) };
    unsafe { aviso_outcome_free(first) };
    // A second wait after completion returns success immediately.
    let second = unsafe { aviso_watch_wait(watch) };
    assert!(unsafe { crate::outcome::aviso_outcome_is_ok(second) });
    unsafe { aviso_outcome_free(second) };
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}
