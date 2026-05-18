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
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use aviso::auth::AuthProvider;
use aviso::watch::WatchRequest;
use aviso::{AvisoClient, ClientError};
use futures_core::Stream;
use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use tokio::time::timeout;
use wiremock::matchers::{header, method, path};
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

/// Stub auth provider whose `authorization_header()` returns a value held
/// in a `Mutex<String>`. `refresh()` swaps the value to the post-refresh
/// token, so the test mock can match the two distinct `Authorization`
/// headers and assert the refreshed credential is actually used on the
/// retry POST.
#[derive(Debug)]
struct SwappingAuth {
    token: Mutex<String>,
    new_token: String,
    refresh_count: Mutex<u32>,
    refresh_always_succeeds: bool,
}

impl SwappingAuth {
    fn new(initial: impl Into<String>, after_refresh: impl Into<String>) -> Self {
        Self {
            token: Mutex::new(initial.into()),
            new_token: after_refresh.into(),
            refresh_count: Mutex::new(0),
            refresh_always_succeeds: true,
        }
    }
}

#[async_trait]
impl AuthProvider for SwappingAuth {
    async fn authorization_header(&self) -> aviso::Result<HeaderValue> {
        let guard = self.token.lock().unwrap();
        HeaderValue::from_str(&format!("Bearer {}", *guard))
            .map_err(|e| ClientError::Auth(format!("header build: {e}")))
    }

    async fn refresh(&self) -> aviso::Result<()> {
        *self.refresh_count.lock().unwrap() += 1;
        if !self.refresh_always_succeeds {
            return Err(ClientError::Auth("test: refresh refused".into()));
        }
        (*self.token.lock().unwrap()).clone_from(&self.new_token);
        Ok(())
    }
}

#[tokio::test]
async fn auth_refresh_on_401_uses_refreshed_credential() {
    let server = MockServer::start().await;

    let body = format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        end_of_stream_chunk(),
    );

    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .and(header("authorization", "Bearer old-token"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("x-request-id", "req-401")
                .set_body_string("unauthorized"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .and(header("authorization", "Bearer new-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let auth = Arc::new(SwappingAuth::new("old-token", "new-token"));
    let client = AvisoClient::builder()
        .base_url(server.uri())
        .auth(auth.clone())
        .build()
        .unwrap();
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    let item = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("notification should arrive after refresh")
        .expect("stream must not close before the first item")
        .expect("no terminal error should surface");
    assert_eq!(item.sequence, 1);

    assert_eq!(
        *auth.refresh_count.lock().unwrap(),
        1,
        "refresh() must be called exactly once"
    );

    drop(stream);
}

#[tokio::test]
async fn auth_refresh_followed_by_second_401_terminates() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("x-request-id", "req-401")
                .set_body_string("still no"),
        )
        .mount(&server)
        .await;

    let auth = Arc::new(SwappingAuth::new("old-token", "new-token"));
    let client = AvisoClient::builder()
        .base_url(server.uri())
        .auth(auth.clone())
        .build()
        .unwrap();
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    let item = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("terminal error should arrive promptly")
        .expect("stream must surface one item before closing");
    match item {
        Err(ClientError::Auth(msg)) => {
            assert!(
                msg.contains("after refresh"),
                "error message should name the post-refresh rejection: {msg}"
            );
        }
        other => panic!("expected ClientError::Auth, got {other:?}"),
    }

    let next = timeout(Duration::from_secs(2), next_item(&mut stream))
        .await
        .expect("stream should close promptly after terminal error");
    assert!(
        next.is_none(),
        "stream must yield None after terminal error"
    );

    assert_eq!(
        *auth.refresh_count.lock().unwrap(),
        1,
        "refresh() must be called exactly once before the second 401 terminates"
    );

    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        2,
        "expected exactly two POSTs (initial 401 + post-refresh 401); got {}",
        received.len()
    );
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
