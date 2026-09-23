// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! End-to-end integration tests for `AvisoClient::watch()` covering the
//! single-connection happy paths: historical-then-live replay, clean close
//! on `end_of_stream`, terminal `MalformedEvent`, and gap detection on a
//! sequence jump.
//!
//! The tests open a `wiremock::MockServer`, register a streaming SSE
//! response, and drive the resulting `NotificationStream` to completion.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::time::Duration;

use aviso::watch::{ResumeStart, WatchRequest};
use aviso::{AvisoClient, ClientError};
use futures_core::Stream;
use serde_json::{Value, json};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
mod common;

fn sse_chunk(event_type: &str, data: &Value) -> String {
    format!("event: {event_type}\ndata: {data}\n\n")
}

fn cloud_event(event_type: &str, sequence: u64) -> Value {
    json!({
        "id": format!("{event_type}@{sequence}"),
        "source": "https://aviso.example",
        "type": format!("int.ecmwf.aviso.{event_type}"),
        "time": "2026-05-17T12:34:56Z",
        "data": {
            "identifier": { "country": "UK" },
            "payload": { "n": sequence }
        }
    })
}

fn replay_completed() -> (&'static str, Value) {
    (
        "replay-control",
        json!({
            "type": "replay_completed",
            "topic": "mars",
            "timestamp": "2026-05-17T12:30:00Z"
        }),
    )
}

fn end_of_stream() -> (&'static str, Value) {
    (
        "connection-closing",
        json!({
            "reason": "end_of_stream",
            "timestamp": "2026-05-17T13:00:00Z",
            "message": "Stream completed",
            "topic": "mars",
            "request_id": "req-eos"
        }),
    )
}

async fn mount_sse_body(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |request: &wiremock::Request| common::opened_sse(request, &body))
        .mount(server)
        .await;
}

fn client_for(server: &MockServer) -> AvisoClient {
    AvisoClient::builder()
        .base_url(server.uri())
        .build()
        .unwrap()
}

async fn collect_stream<S>(mut stream: S) -> Vec<Result<aviso::Notification, ClientError>>
where
    S: Stream<Item = Result<aviso::Notification, ClientError>> + Unpin,
{
    let mut out = Vec::new();
    // Hand-rolled drain because we deliberately do not depend on
    // `futures-util`; `poll_next` over `Pin<&mut S>` is enough.
    loop {
        let item = std::future::poll_fn(|cx| {
            let pinned = std::pin::Pin::new(&mut stream);
            <S as Stream>::poll_next(pinned, cx)
        })
        .await;
        match item {
            Some(v) => out.push(v),
            None => break,
        }
    }
    out
}

async fn take_n<S>(mut stream: S, n: usize) -> (Vec<Result<aviso::Notification, ClientError>>, S)
where
    S: Stream<Item = Result<aviso::Notification, ClientError>> + Unpin,
{
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let item = std::future::poll_fn(|cx| {
            let pinned = std::pin::Pin::new(&mut stream);
            <S as Stream>::poll_next(pinned, cx)
        })
        .await;
        match item {
            Some(v) => out.push(v),
            None => break,
        }
    }
    (out, stream)
}

#[tokio::test]
async fn watch_happy_path_replay_then_live() {
    // Watch mode: replay items then live items. With the resilience layer
    // in place, end_of_stream triggers a reconnect rather than closing
    // the stream, so the test takes the expected first four items and
    // drops the stream to break the (now-infinite) reconnect loop.
    let server = MockServer::start().await;
    let (rc_event, rc_data) = replay_completed();
    let (eos_event, eos_data) = end_of_stream();
    let body = format!(
        "{}{}{}{}{}{}",
        sse_chunk("replay", &cloud_event("mars", 10)),
        sse_chunk("replay", &cloud_event("mars", 11)),
        sse_chunk(rc_event, &rc_data),
        sse_chunk("live-notification", &cloud_event("mars", 12)),
        sse_chunk("live-notification", &cloud_event("mars", 13)),
        sse_chunk(eos_event, &eos_data),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let stream = client
        .watch(WatchRequest::watch_from(
            "mars",
            ResumeStart::AfterSequence(9),
        ))
        .unwrap();
    let (items, stream) = timeout(Duration::from_secs(5), take_n(stream, 4))
        .await
        .expect("first four items should arrive promptly");

    let sequences: Vec<u64> = items
        .iter()
        .map(|item| item.as_ref().expect("all items should be Ok").sequence)
        .collect();
    assert_eq!(sequences, vec![10, 11, 12, 13]);
    drop(stream);
}

#[tokio::test]
async fn watch_first_notification_arrives_in_watch_mode() {
    // Watch mode: server emits one notification then end_of_stream. The
    // notification surfaces; end_of_stream triggers a reconnect rather
    // than closing the stream. The test asserts on the first item and
    // drops the stream.
    let server = MockServer::start().await;
    let (eos_event, eos_data) = end_of_stream();
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        sse_chunk(eos_event, &eos_data),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let (items, stream) = timeout(Duration::from_secs(5), take_n(stream, 1))
        .await
        .expect("first item should arrive promptly");
    assert_eq!(items.len(), 1);
    assert!(items[0].is_ok(), "got {:?}", items[0]);
    drop(stream);
}

#[tokio::test]
async fn watch_malformed_cloudevent_id_terminates_stream_with_typed_error() {
    let server = MockServer::start().await;
    let bad = json!({
        "id": "mars@notanumber",
        "source": "https://aviso.example",
        "type": "int.ecmwf.aviso.mars",
        "time": "2026-05-17T12:34:56Z",
        "data": { "identifier": {}, "payload": null }
    });
    mount_sse_body(&server, sse_chunk("live-notification", &bad)).await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should terminate promptly");

    assert_eq!(items.len(), 1);
    match &items[0] {
        Err(ClientError::MalformedEvent(_)) => {}
        other => panic!("expected MalformedEvent, got {other:?}"),
    }
}

#[tokio::test]
async fn watch_rejects_a_notification_for_another_event_type() {
    // The server is trusted to send only the stream that was asked for.
    // One that sends something else is treated like one that sends a
    // malformed id: the notification never reaches the caller or the
    // triggers, and the watch ends rather than reconnecting into the
    // same stream.
    let server = MockServer::start().await;
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 10)),
        sse_chunk("live-notification", &cloud_event("cosmo", 11)),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should terminate promptly");

    assert_eq!(items.len(), 2, "got: {items:?}");
    assert_eq!(items[0].as_ref().unwrap().event_type, "mars");
    match &items[1] {
        Err(ClientError::MalformedEvent(message)) => {
            assert!(message.contains("\"cosmo\""), "got: {message}");
            assert!(message.contains("\"mars\""), "got: {message}");
        }
        other => panic!("expected MalformedEvent, got {other:?}"),
    }
}

#[tokio::test]
async fn watch_ends_with_a_protocol_error_when_a_line_never_terminates() {
    // A server that streams bytes with no line ending would otherwise be
    // held in memory for as long as it kept sending. The parser stops at
    // its bound and the watch reports it as a protocol violation rather
    // than reconnecting into the same stream.
    let server = MockServer::start().await;
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 10)),
        "x".repeat(2 * 1024 * 1024),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let items = timeout(Duration::from_secs(10), collect_stream(stream))
        .await
        .expect("stream should terminate promptly");

    assert_eq!(items.len(), 2, "got: {items:?}");
    assert_eq!(items[0].as_ref().unwrap().sequence, 10);
    match &items[1] {
        Err(ClientError::StreamProtocol { message, .. }) => {
            assert!(message.contains("SSE line exceeds"), "got: {message}");
        }
        other => panic!("expected StreamProtocol, got {other:?}"),
    }
}

#[tokio::test]
async fn a_notification_completed_before_the_overflow_is_still_delivered() {
    // Whether the transport hands over the notification and the endless
    // line together or apart, the notification was complete before the
    // bound was hit and must reach the caller before the watch ends.
    let server = MockServer::start().await;
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 7)),
        "y".repeat(2 * 1024 * 1024),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let items = timeout(Duration::from_secs(10), collect_stream(stream))
        .await
        .expect("stream should terminate promptly");

    assert_eq!(items.len(), 2, "got: {items:?}");
    assert_eq!(items[0].as_ref().unwrap().sequence, 7);
    assert!(matches!(items[1], Err(ClientError::StreamProtocol { .. })));
}

#[tokio::test]
async fn an_overflow_before_the_stream_is_confirmed_is_reported() {
    // The server never sends the frame that confirms the subscription;
    // it sends an endless line instead. The watch must report the
    // overflow rather than wait for a confirmation that cannot come.
    let server = MockServer::start().await;
    // Not `mount_sse_body`: that helper prepends the confirmation frame.
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("z".repeat(2 * 1024 * 1024), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should terminate promptly, not wait for the opening deadline");

    assert_eq!(items.len(), 1, "got: {items:?}");
    match &items[0] {
        Err(ClientError::StreamProtocol { message, .. }) => {
            assert!(message.contains("SSE line exceeds"), "got: {message}");
        }
        other => panic!("expected StreamProtocol, got {other:?}"),
    }
}

#[tokio::test]
async fn watch_gap_detection_on_sequence_jump_terminates_with_history_gap() {
    let server = MockServer::start().await;
    let (eos_event, eos_data) = end_of_stream();
    // 10, 11, then jump to 13 (12 missing). Append end_of_stream so the
    // server-side response completes cleanly; the supervisor terminates on
    // the gap before reaching it.
    let body = format!(
        "{}{}{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 10)),
        sse_chunk("live-notification", &cloud_event("mars", 11)),
        sse_chunk("live-notification", &cloud_event("mars", 13)),
        sse_chunk(eos_event, &eos_data),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let stream = client
        .watch(WatchRequest::watch_from(
            "mars",
            ResumeStart::AfterSequence(9),
        ))
        .unwrap();
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should drain promptly");

    assert_eq!(items.len(), 3, "two Ok(_) plus one Err(HistoryGap)");
    assert!(items[0].as_ref().unwrap().sequence == 10);
    assert!(items[1].as_ref().unwrap().sequence == 11);
    match &items[2] {
        Err(ClientError::HistoryGap {
            reason: aviso::watch::GapReason::SequenceJump { expected, observed },
        }) => {
            assert_eq!(*expected, 12);
            assert_eq!(*observed, 13);
        }
        other => panic!("expected HistoryGap{{SequenceJump}}, got {other:?}"),
    }
}

#[test]
fn watch_returns_config_error_when_no_tokio_runtime() {
    // `AvisoClient::watch` is sync. Called from a plain `#[test]` (no
    // Tokio runtime), `Handle::try_current()` returns `Err(_)` and the
    // method must surface that as `ClientError::Config`, never panic.
    // The client is built with a placeholder URL because the request is
    // rejected before any I/O is attempted.
    let client = AvisoClient::builder()
        .base_url("http://127.0.0.1:0")
        .build()
        .unwrap();
    let err = client.watch(WatchRequest::watch("mars")).unwrap_err();
    assert!(
        matches!(err, ClientError::Config(_)),
        "expected Config error, got {err:?}"
    );
}

#[tokio::test]
async fn watch_with_handler_observes_first_four_items_in_replay_then_live() {
    // Watch mode: replay then live. end_of_stream reconnects under the
    // resilience layer; the handler runs forever in that case. The test
    // gives up the handler after the fourth item by returning `Err(_)`,
    // which cancels the supervisor cleanly.
    let server = MockServer::start().await;
    let (rc_event, rc_data) = replay_completed();
    let (eos_event, eos_data) = end_of_stream();
    let body = format!(
        "{}{}{}{}{}{}",
        sse_chunk("replay", &cloud_event("mars", 10)),
        sse_chunk("replay", &cloud_event("mars", 11)),
        sse_chunk(rc_event, &rc_data),
        sse_chunk("live-notification", &cloud_event("mars", 12)),
        sse_chunk("live-notification", &cloud_event("mars", 13)),
        sse_chunk(eos_event, &eos_data),
    );
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
    let observed_clone = observed.clone();
    let result = timeout(
        Duration::from_secs(5),
        client.watch_with_handler(
            WatchRequest::watch_from("mars", ResumeStart::AfterSequence(9)),
            move |notification| {
                let observed = observed_clone.clone();
                async move {
                    let mut guard = observed.lock().unwrap();
                    guard.push(notification.sequence);
                    if guard.len() >= 4 {
                        Err(ClientError::Config("test-stop".into()))
                    } else {
                        Ok(())
                    }
                }
            },
        ),
    )
    .await
    .expect("handler loop should finish promptly");
    match result {
        Err(ClientError::Config(msg)) => assert_eq!(msg, "test-stop"),
        other => panic!("expected the sentinel Config error, got {other:?}"),
    }
    assert_eq!(observed.lock().unwrap().clone(), vec![10, 11, 12, 13]);
}

#[tokio::test]
async fn watch_with_handler_propagates_handler_error_and_cancels_supervisor() {
    let server = MockServer::start().await;
    // The wire has plenty of notifications; the handler returns Err after
    // the first item, which must propagate as the method's return value
    // AND tear the supervisor down (no test-runtime hang).
    let mut body = String::new();
    for n in 1..=10 {
        body.push_str(&sse_chunk("live-notification", &cloud_event("mars", n)));
    }
    mount_sse_body(&server, body).await;

    let client = client_for(&server);
    let err = timeout(
        Duration::from_secs(5),
        client.watch_with_handler(WatchRequest::watch("mars"), |_n| async {
            Err(ClientError::Config("handler refused".into()))
        }),
    )
    .await
    .expect("propagation should complete promptly")
    .unwrap_err();
    assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
}

#[tokio::test]
async fn watch_returns_http_error_for_terminal_non_success_status() {
    // 404 is terminal under the resilience layer's classification (other
    // 4xx). 503 is retryable and reconnects, so it is exercised by the
    // dedicated retry-after integration test instead.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(404)
                .insert_header("x-request-id", "req-gone")
                .set_body_string("not found"),
        )
        .mount(&server)
        .await;

    let client = client_for(&server);
    let stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should terminate promptly");

    assert_eq!(items.len(), 1);
    match &items[0] {
        Err(ClientError::Http {
            status,
            body,
            request_id,
        }) => {
            assert_eq!(*status, 404);
            assert_eq!(body, "not found");
            assert_eq!(request_id.as_deref(), Some("req-gone"));
        }
        other => panic!("expected Http(404), got {other:?}"),
    }
}
