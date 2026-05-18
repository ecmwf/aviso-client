//! Integration tests for `AvisoClient::watch()` resilience behaviour:
//! reconnect on routine server close, `Retry-After` honouring on 503,
//! and EOF-without-close reclassification as routine transport loss.
//!
//! Auth refresh, heartbeat watchdog, state-store-backed checkpoints, and
//! the parent-drop cancel cascade ship in follow-up commits and have
//! dedicated tests there.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::pin::Pin;
use std::time::{Duration, Instant};

use aviso::AvisoClient;
use aviso::watch::WatchRequest;
use futures_core::Stream;
use serde_json::{Value, json};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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

fn end_of_stream_chunk() -> String {
    sse_chunk(
        "connection-closing",
        &json!({
            "reason": "end_of_stream",
            "timestamp": "2026-05-17T13:00:00Z",
            "message": "",
            "topic": "mars",
            "request_id": "req-eos"
        }),
    )
}

fn max_duration_chunk() -> String {
    sse_chunk(
        "connection-closing",
        &json!({
            "reason": "max_duration_reached",
            "timestamp": "2026-05-17T13:00:00Z",
            "message": "",
            "topic": "mars",
            "request_id": "req-max"
        }),
    )
}

fn client_for(server: &MockServer) -> AvisoClient {
    AvisoClient::builder()
        .base_url(server.uri())
        .build()
        .unwrap()
}

async fn next_item<S>(stream: &mut S) -> Option<S::Item>
where
    S: Stream + Unpin,
{
    std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

#[tokio::test]
async fn reconnect_after_max_duration_reached() {
    // Server returns a stream that ends with `max_duration_reached`. The
    // supervisor must reconnect and serve the next iteration's notifications
    // without the test ever seeing a terminal error.
    let server = MockServer::start().await;
    let body = format!(
        "{}{}{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        sse_chunk("live-notification", &cloud_event("mars", 2)),
        sse_chunk("live-notification", &cloud_event("mars", 3)),
        max_duration_chunk(),
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let client = client_for(&server);
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    // Collect five notifications. The first three come from the initial
    // connection; the second pair come from the reconnect that fires
    // after `max_duration_reached`. The mock serves the same body to
    // every POST, so sequences 1, 2, 3 are re-delivered.
    let mut sequences = Vec::new();
    for _ in 0..5 {
        let item = timeout(Duration::from_secs(5), next_item(&mut stream))
            .await
            .expect("each notification should arrive within 5s")
            .expect("stream must not close before five items")
            .expect("no terminal error should surface");
        sequences.push(item.sequence);
    }

    assert_eq!(
        &sequences[0..3],
        &[1, 2, 3],
        "first three from initial connection"
    );
    // The second connection re-serves the body, so sequences[3..5] are
    // also from {1, 2, 3} (the mock does not differentiate by request
    // count). The point of the test is that reconnect happens at all;
    // exact sequence values are an artefact of mock simplicity.
    assert!(
        sequences.len() == 5,
        "five notifications must arrive across two connections"
    );

    let received_requests = server.received_requests().await.unwrap();
    assert!(
        received_requests.len() >= 2,
        "at least two POSTs (initial + reconnect); got {}",
        received_requests.len()
    );

    drop(stream);
}

#[tokio::test]
async fn retry_after_honoured_on_503() {
    // First POST returns 503 with `Retry-After: 1`; second POST returns a
    // 200 stream with one notification. The test asserts the wall-clock
    // delta between the two POSTs is at least the retry-after value.
    use std::sync::Mutex;
    let server = MockServer::start().await;
    let attempt = std::sync::Arc::new(Mutex::new(0u32));

    let body = format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        end_of_stream_chunk(),
    );

    let body_clone = body.clone();
    let attempt_clone = attempt.clone();
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |_: &Request| {
            let mut a = attempt_clone.lock().unwrap();
            *a += 1;
            if *a == 1 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after", "1")
                    .set_body_string("busy")
            } else {
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body_clone.clone())
            }
        })
        .mount(&server)
        .await;

    let client = client_for(&server);
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    let start = Instant::now();
    let item = timeout(Duration::from_secs(10), next_item(&mut stream))
        .await
        .expect("notification should arrive within 10s")
        .expect("stream must not close before first item")
        .expect("no terminal error should surface");
    let elapsed = start.elapsed();
    assert_eq!(item.sequence, 1);
    assert!(
        elapsed >= Duration::from_secs(1),
        "wall-clock between POSTs must be at least the Retry-After value (1s); got {elapsed:?}"
    );

    let received = server.received_requests().await.unwrap();
    assert!(
        received.len() >= 2,
        "expected at least two POSTs; got {}",
        received.len()
    );

    drop(stream);
}

#[tokio::test]
async fn unexpected_eof_triggers_reconnect() {
    // Server returns a stream with one notification followed by a clean
    // TCP close (no `connection-closing` frame). The supervisor must
    // reconnect rather than surface a terminal `StreamProtocol` error.
    let server = MockServer::start().await;
    let body = sse_chunk("live-notification", &cloud_event("mars", 1));
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let client = client_for(&server);
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    let item = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("first notification should arrive promptly")
        .expect("stream must not close before first item")
        .expect("no terminal error must surface on EOF");
    assert_eq!(item.sequence, 1);

    // Give the supervisor a window to reconnect and serve the second
    // POST's body. The mock returns the same body to every POST, so the
    // second item (whatever its sequence) proves reconnect happened.
    let second = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("reconnect should re-serve the body and emit a second notification")
        .expect("stream must not close")
        .expect("no terminal error on the second connection");
    let _ = second;

    let received = server.received_requests().await.unwrap();
    assert!(
        received.len() >= 2,
        "EOF must trigger reconnect (at least two POSTs); got {}",
        received.len()
    );

    drop(stream);
}
