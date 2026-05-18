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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
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

#[tokio::test]
async fn watch_happy_path_replay_then_live() {
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
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should drain promptly");

    let sequences: Vec<u64> = items
        .iter()
        .map(|item| item.as_ref().expect("all items should be Ok").sequence)
        .collect();
    assert_eq!(sequences, vec![10, 11, 12, 13]);
}

#[tokio::test]
async fn watch_clean_close_on_end_of_stream_yields_no_error() {
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
    let items = timeout(Duration::from_secs(5), collect_stream(stream))
        .await
        .expect("stream should drain promptly");

    assert_eq!(items.len(), 1, "exactly one Ok(_), then None");
    assert!(items[0].is_ok(), "got {:?}", items[0]);
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
async fn watch_returns_http_error_for_non_success_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("x-request-id", "req-busy")
                .set_body_string("server busy"),
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
            assert_eq!(*status, 503);
            assert_eq!(body, "server busy");
            assert_eq!(request_id.as_deref(), Some("req-busy"));
        }
        other => panic!("expected Http(503), got {other:?}"),
    }
}
