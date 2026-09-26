// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `aviso_client_watch_many` through its C ABI, against a mock server that
//! answers each watch according to its event type.

#![allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "test code: every unsafe block here is a call into the crate's own C ABI from Rust, with the arguments constructed a few lines above"
)]

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::super::handle::{aviso_watch_free, aviso_watch_stop, aviso_watch_wait};
use super::super::notification::aviso_notification_sequence;
use super::super::request::{
    aviso_watch_request_new, aviso_watch_request_replay_from_sequence,
    aviso_watch_request_set_filter_json,
};
use super::*;
use crate::AvisoErrorKind;
use crate::client::{aviso_client_builder_build, aviso_client_builder_new, aviso_client_free};
use crate::outcome::{aviso_outcome_error, aviso_outcome_free, aviso_outcome_take_client};

/// How a test's `on_end` saw the watch finish: `None` for success, else the
/// error kind and message.
type End = Option<(AvisoErrorKind, String)>;

/// What the callbacks saw, shared with the test through `ctx`.
struct Sink {
    /// What `on_error` returns: `true` keeps the other watches running.
    continue_on_error: bool,
    /// `on_notification` returns `false` once this many have arrived.
    stop_after: Option<usize>,
    notifications: Mutex<Vec<(String, u64)>>,
    errors: Mutex<Vec<(String, AvisoErrorKind, u16, String)>>,
    ends: AtomicUsize,
    end: Mutex<Option<End>>,
}

impl Sink {
    fn new(continue_on_error: bool) -> Self {
        Self {
            continue_on_error,
            stop_after: None,
            notifications: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            ends: AtomicUsize::new(0),
            end: Mutex::new(None),
        }
    }

    fn ctx(&self) -> *mut c_void {
        (&raw const *self).cast_mut().cast()
    }

    fn sequences(&self, name: &str) -> Vec<u64> {
        self.notifications
            .lock()
            .expect("notifications lock")
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, sequence)| *sequence)
            .collect()
    }

    fn end(&self) -> End {
        assert_eq!(self.ends.load(Ordering::SeqCst), 1, "on_end fires once");
        self.end
            .lock()
            .expect("end lock")
            .clone()
            .expect("on_end has fired")
    }
}

fn text(ptr: *const c_char) -> String {
    assert!(!ptr.is_null());
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

extern "C" fn on_notification(
    ctx: *mut c_void,
    name: *const c_char,
    notification: *const AvisoNotification,
) -> bool {
    let sink = unsafe { &*(ctx as *const Sink) };
    let sequence = unsafe { aviso_notification_sequence(notification) };
    let mut seen = sink.notifications.lock().expect("notifications lock");
    seen.push((text(name), sequence));
    sink.stop_after.is_none_or(|limit| seen.len() < limit)
}

extern "C" fn on_error(ctx: *mut c_void, name: *const c_char, error: *const AvisoError) -> bool {
    let sink = unsafe { &*(ctx as *const Sink) };
    let error = unsafe { &*error };
    sink.errors.lock().expect("errors lock").push((
        text(name),
        error.kind,
        error.http_status,
        text(error.message),
    ));
    sink.continue_on_error
}

extern "C" fn on_end(ctx: *mut c_void, outcome: *mut AvisoOutcome) {
    let sink = unsafe { &*(ctx as *const Sink) };
    let error = unsafe { aviso_outcome_error(outcome) };
    let end = if error.is_null() {
        None
    } else {
        let error = unsafe { &*error };
        Some((error.kind, text(error.message)))
    };
    *sink.end.lock().expect("end lock") = Some(end);
    sink.ends.fetch_add(1, Ordering::SeqCst);
    unsafe { aviso_outcome_free(outcome) };
}

fn cstr(value: &str) -> CString {
    CString::new(value).expect("cstring")
}

fn build_client(base_url: &str) -> *mut AvisoClient {
    let base = cstr(base_url);
    let mut builder = unsafe { aviso_client_builder_new(base.as_ptr()) };
    let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
    let client = unsafe { aviso_outcome_take_client(outcome) };
    unsafe { aviso_outcome_free(outcome) };
    assert!(!client.is_null());
    client
}

/// A request that replays `event_type` from the start and then ends.
fn replay(event_type: &str) -> *mut AvisoWatchRequest {
    let event = cstr(event_type);
    let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
    unsafe { aviso_watch_request_replay_from_sequence(request, 0) };
    request
}

/// A live request for `event_type`, which runs until it is stopped.
fn live(event_type: &str) -> *mut AvisoWatchRequest {
    let event = cstr(event_type);
    unsafe { aviso_watch_request_new(event.as_ptr()) }
}

fn list_of(entries: Vec<(&str, *mut AvisoWatchRequest)>) -> *mut AvisoWatchList {
    let list = aviso_watch_list_new();
    for (name, mut request) in entries {
        let name = cstr(name);
        unsafe { aviso_watch_list_add(list, name.as_ptr(), &raw mut request) };
        assert!(request.is_null(), "the request pointer must be nulled");
    }
    list
}

fn start(
    client: *const AvisoClient,
    mut list: *mut AvisoWatchList,
    sink: &Sink,
) -> *mut AvisoWatch {
    let watch = unsafe {
        aviso_client_watch_many(
            client,
            &raw mut list,
            Some(on_notification),
            Some(on_error),
            Some(on_end),
            sink.ctx(),
        )
    };
    assert!(!watch.is_null());
    assert!(list.is_null(), "the list pointer must be nulled");
    watch
}

/// Waits for the watch to end, with a bound so a watch that never ends fails
/// the test instead of hanging it.
fn wait(watch: *mut AvisoWatch) {
    let (done, finished) = mpsc::channel();
    let handle = SendPtr(watch.cast());
    std::thread::spawn(move || {
        let handle = handle;
        let outcome = unsafe { aviso_watch_wait(handle.0.cast()) };
        let ok = unsafe { aviso_outcome_error(outcome) }.is_null();
        unsafe { aviso_outcome_free(outcome) };
        done.send(ok).expect("send");
    });
    let ok = finished
        .recv_timeout(Duration::from_secs(60))
        .expect("the merged watch should end on its own");
    assert!(ok, "wait succeeds from a thread outside the runtime");
}

fn sse(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// Opens the stream, delivers `count` notifications, and closes it.
fn finished_replay(event_type: &str, count: u64) -> String {
    let sequences: Vec<u64> = (1..=count).collect();
    replay_of(event_type, &sequences)
}

/// Opens the stream, delivers notifications with the given sequence numbers,
/// and closes it.
fn replay_of(event_type: &str, sequences: &[u64]) -> String {
    let mut body = sse("replay-control", &json!({"type": "replay_started"}));
    for &sequence in sequences {
        body.push_str(&sse(
            "replay",
            &json!({
                "id": format!("{event_type}@{sequence}"),
                "source": "https://aviso.example",
                "type": format!("int.ecmwf.aviso.{event_type}"),
                "time": "2026-05-17T12:34:56Z",
                "data": { "identifier": {}, "payload": null }
            }),
        ));
    }
    body.push_str(&sse(
        "replay-control",
        &json!({"type": "replay_completed", "topic": event_type,
                "timestamp": "2026-05-17T12:30:00Z"}),
    ));
    body.push_str(&sse(
        "connection-closing",
        &json!({"reason": "end_of_stream", "timestamp": "2026-05-17T13:00:00Z",
                "message": "done", "topic": event_type, "request_id": "r"}),
    ));
    body
}

/// Mounts a response for the watches of `event_type` on the mock server.
fn answer(server: &MockServer, event_type: &str, response: ResponseTemplate) {
    crate::runtime().block_on(
        Mock::given(method("POST"))
            .and(path_regex("^/api/v1/(watch|replay)$"))
            .and(body_partial_json(json!({ "event_type": event_type })))
            .respond_with(response)
            .mount(server),
    );
}

fn serve(server: &MockServer, event_type: &str, count: u64) {
    let body = finished_replay(event_type, count);
    answer(
        server,
        event_type,
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"),
    );
}

/// Replays sequences 1 and 3: the missing 2 is a history gap.
fn serve_with_gap(server: &MockServer, event_type: &str) {
    let body = replay_of(event_type, &[1, 3]);
    answer(
        server,
        event_type,
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"),
    );
}

fn refuse(server: &MockServer, event_type: &str) {
    answer(
        server,
        event_type,
        ResponseTemplate::new(400).set_body_string("unknown field 'stepp'"),
    );
}

fn mock_server() -> MockServer {
    crate::runtime().block_on(MockServer::start())
}

#[test]
fn each_notification_carries_its_watch_name_and_the_watch_ends_when_all_do() {
    let server = mock_server();
    serve(&server, "alpha", 2);
    serve(&server, "beta", 1);
    let client = build_client(&server.uri());
    let sink = Sink::new(false);

    let list = list_of(vec![("surface", replay("alpha")), ("wave", replay("beta"))]);
    let watch = start(client, list, &sink);
    wait(watch);

    assert_eq!(sink.sequences("surface"), [1, 2]);
    assert_eq!(sink.sequences("wave"), [1]);
    assert!(sink.errors.lock().expect("errors lock").is_empty());
    assert_eq!(sink.end(), None);
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn on_error_returning_true_drops_the_failed_watch_and_keeps_the_others() {
    let server = mock_server();
    serve(&server, "alpha", 2);
    refuse(&server, "beta");
    let client = build_client(&server.uri());
    let sink = Sink::new(true);

    let list = list_of(vec![("surface", replay("alpha")), ("typo", replay("beta"))]);
    let watch = start(client, list, &sink);
    wait(watch);

    assert_eq!(sink.sequences("surface"), [1, 2]);
    let errors = sink.errors.lock().expect("errors lock");
    assert_eq!(errors.len(), 1, "one failure: {errors:?}");
    let (name, kind, status, message) = &errors[0];
    assert_eq!(name, "typo");
    assert_eq!(*kind, AvisoErrorKind::Http);
    assert_eq!(*status, 400);
    assert!(message.starts_with("watch 'typo': http 400"), "{message}");
    drop(errors);
    assert_eq!(sink.end(), None, "a watch still succeeded");
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn on_error_returning_false_stops_every_watch_and_reports_the_error() {
    let server = mock_server();
    refuse(&server, "alpha");
    // A live watch whose stream never ends on its own: the merged watch can
    // only end because the failure stops it.
    answer(
        &server,
        "beta",
        ResponseTemplate::new(200).set_body_raw(
            sse(
                "live-notification",
                &json!({"type": "connection_established"}),
            ),
            "text/event-stream",
        ),
    );
    let client = build_client(&server.uri());
    let sink = Sink::new(false);

    let list = list_of(vec![("typo", replay("alpha")), ("live", live("beta"))]);
    let watch = start(client, list, &sink);
    wait(watch);

    assert_eq!(sink.errors.lock().expect("errors lock").len(), 1);
    let (kind, message) = sink.end().expect("the failure ends the watch");
    assert_eq!(kind, AvisoErrorKind::Http);
    assert!(message.starts_with("watch 'typo': http 400"), "{message}");
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn every_watch_failing_ends_with_an_error_listing_each_failure() {
    let server = mock_server();
    refuse(&server, "alpha");
    serve_with_gap(&server, "beta");
    let client = build_client(&server.uri());
    let sink = Sink::new(true);

    let list = list_of(vec![("refused", replay("alpha")), ("gap", replay("beta"))]);
    let watch = start(client, list, &sink);
    wait(watch);

    let kinds: Vec<AvisoErrorKind> = sink
        .errors
        .lock()
        .expect("errors lock")
        .iter()
        .map(|(_, kind, _, _)| *kind)
        .collect();
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(kinds.contains(&AvisoErrorKind::Http), "{kinds:?}");
    assert!(kinds.contains(&AvisoErrorKind::HistoryGap), "{kinds:?}");
    let (kind, message) = sink.end().expect("every watch failed");
    // The two failures arrive in either order; the end keeps the kind of
    // the one reported last.
    assert_eq!(Some(&kind), kinds.last());
    assert!(
        message.starts_with("every watch failed: watch '"),
        "{message}"
    );
    assert!(message.contains("watch 'refused': http 400"), "{message}");
    assert!(message.contains("watch 'gap': history gap"), "{message}");
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn on_notification_returning_false_stops_every_watch_without_an_error() {
    let server = mock_server();
    serve(&server, "alpha", 5);
    // A live watch that never ends on its own: the merged watch can only end
    // because on_notification asked it to.
    answer(
        &server,
        "beta",
        ResponseTemplate::new(200).set_body_raw(
            sse(
                "live-notification",
                &json!({"type": "connection_established"}),
            ),
            "text/event-stream",
        ),
    );
    let client = build_client(&server.uri());
    let mut sink = Sink::new(false);
    sink.stop_after = Some(1);

    let list = list_of(vec![("replay", replay("alpha")), ("live", live("beta"))]);
    let watch = start(client, list, &sink);
    wait(watch);

    assert_eq!(sink.sequences("replay"), [1], "nothing after the stop");
    assert_eq!(sink.end(), None);
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

#[test]
fn a_stop_from_outside_ends_every_watch_without_an_error() {
    // No server: both live watches keep retrying until they are stopped.
    let client = build_client("http://127.0.0.1:1");
    let sink = Sink::new(false);

    let list = list_of(vec![("one", live("alpha")), ("two", live("beta"))]);
    let watch = start(client, list, &sink);
    unsafe { aviso_watch_stop(watch) };
    wait(watch);

    assert_eq!(sink.end(), None);
    assert!(sink.errors.lock().expect("errors lock").is_empty());
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
}

/// Starts a merged watch over `list` against an unreachable server and
/// returns the error `on_end` received.
fn start_error(list: *mut AvisoWatchList) -> (AvisoErrorKind, String) {
    let client = build_client("http://127.0.0.1:1");
    let sink = Sink::new(false);
    let watch = start(client, list, &sink);
    wait(watch);
    assert!(sink.notifications.lock().expect("lock").is_empty());
    let end = sink.end().expect("the list is refused");
    unsafe { aviso_watch_free(watch) };
    unsafe { aviso_client_free(client) };
    end
}

#[test]
fn an_invalid_list_is_reported_through_on_end_before_any_watch_opens() {
    let (kind, message) = start_error(list_of(vec![]));
    assert_eq!(kind, AvisoErrorKind::Config);
    assert!(message.contains("at least one request"), "{message}");

    let (kind, message) = start_error(list_of(vec![("same", live("a")), ("same", live("b"))]));
    assert_eq!(kind, AvisoErrorKind::Config);
    assert!(message.contains("'same' is used twice"), "{message}");

    let bad_filter = live("a");
    let filter = cstr("[1, 2]");
    unsafe { aviso_watch_request_set_filter_json(bad_filter, filter.as_ptr()) };
    let (kind, message) = start_error(list_of(vec![("ok", live("b")), ("bad", bad_filter)]));
    assert_eq!(kind, AvisoErrorKind::InvalidInput);
    assert!(message.starts_with("watch 'bad': filter_json"), "{message}");
}

#[test]
fn list_add_consumes_the_request_on_every_path() {
    // A null name: the request is still taken and the error remembered.
    let list = aviso_watch_list_new();
    let mut request = live("a");
    unsafe { aviso_watch_list_add(list, ptr::null(), &raw mut request) };
    assert!(request.is_null());
    let (kind, message) = start_error(list);
    assert_eq!(kind, AvisoErrorKind::InvalidInput);
    assert!(message.contains("watch name"), "{message}");

    // A null list: the request is taken and freed.
    let mut request = live("a");
    let name = cstr("x");
    unsafe { aviso_watch_list_add(ptr::null_mut(), name.as_ptr(), &raw mut request) };
    assert!(request.is_null());

    // A null request slot is recorded against the name.
    let list = aviso_watch_list_new();
    let mut empty: *mut AvisoWatchRequest = ptr::null_mut();
    unsafe { aviso_watch_list_add(list, name.as_ptr(), &raw mut empty) };
    let (kind, message) = start_error(list);
    assert_eq!(kind, AvisoErrorKind::InvalidInput);
    assert!(message.starts_with("watch 'x':"), "{message}");

    // An unused list frees its requests.
    let list = list_of(vec![("a", live("a"))]);
    unsafe { aviso_watch_list_free(list) };
}

#[test]
fn null_arguments_start_nothing_and_still_consume_the_list() {
    let client = build_client("http://127.0.0.1:1");
    let mut list = list_of(vec![("a", live("a"))]);
    let watch = unsafe {
        aviso_client_watch_many(
            client,
            &raw mut list,
            Some(on_notification),
            None,
            Some(on_end),
            ptr::null_mut(),
        )
    };
    assert!(watch.is_null());
    assert!(list.is_null(), "the list pointer must be nulled");

    let null_slot = unsafe {
        aviso_client_watch_many(
            client,
            ptr::null_mut(),
            Some(on_notification),
            Some(on_error),
            Some(on_end),
            ptr::null_mut(),
        )
    };
    assert!(null_slot.is_null());
    unsafe { aviso_client_free(client) };
}
