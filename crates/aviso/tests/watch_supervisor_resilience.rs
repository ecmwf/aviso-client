//! Integration tests for `AvisoClient::watch()` resilience behaviour:
//! reconnect on routine server close, `Retry-After` honouring on 503,
//! EOF-without-close reclassification as routine transport loss, auth
//! refresh on 401, heartbeat-starvation reconnect, state-store-backed
//! checkpoint round-trip, user-`from` precedence over the stored
//! checkpoint, and parent-drop cancel cascade including the
//! channel-full edge case.

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
use aviso::state::MemoryStore;
use aviso::watch::{ResumeStart, WatchRequest};
use aviso::{AvisoClient, ClientError};
use futures_core::Stream;
use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
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

/// Start a tokio `TcpListener` that accepts HTTP connections, reads (and
/// discards) the request, writes a 200 SSE response with the configured
/// per-connection body, and then holds the connection open without
/// further writes until the client closes it. The handler indexes per
/// accepted connection so the test can serve different bodies on the
/// first vs second POST.
async fn paced_sse_server(bodies: Vec<String>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        let mut index = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = bodies.get(index).cloned().unwrap_or_default();
            index += 1;
            tokio::spawn(async move {
                let mut request_buf = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request_buf).await;
                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: \
                     chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;
                // Write the body as a single HTTP chunked frame, then hold
                // the connection open without writing the terminating
                // zero-length chunk so the consumer's `response.chunk()`
                // blocks (as a real long-lived SSE stream would).
                let chunk = format!("{:X}\r\n{}\r\n", body.len(), body);
                let _ = socket.write_all(chunk.as_bytes()).await;
                let _ = socket.flush().await;
                // Hold the connection open. The supervisor's heartbeat
                // watchdog or per-stream cancel breaks the loop.
                let mut sink = [0u8; 256];
                loop {
                    match tokio::io::AsyncReadExt::read(&mut socket, &mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
        }
    });
    (url, handle)
}

#[tokio::test]
async fn heartbeat_starvation_triggers_reconnect() {
    // Real time for the initial connect and first read so HTTP I/O
    // actually delivers bytes; then pause + advance to drive the
    // 90-second heartbeat watchdog deterministically without waiting in
    // wall-clock time. Auto-advance via `start_paused = true` would fire
    // the budget before the supervisor ever connects, so we pause only
    // after the first item arrives.
    let bodies = vec![
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        sse_chunk("live-notification", &cloud_event("mars", 2)),
    ];
    let (url, _server_handle) = paced_sse_server(bodies).await;

    let client = AvisoClient::builder().base_url(&url).build().unwrap();
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    let first = timeout(Duration::from_secs(10), next_item(&mut stream))
        .await
        .expect("first notification must arrive in wall-clock time")
        .expect("stream must not close")
        .expect("no terminal error");
    assert_eq!(first.sequence, 1);

    // Now pause and advance past the 90-second budget. The supervisor's
    // `tokio::time::timeout(budget, response.chunk())` arms its timer in
    // virtual time; advancing fires it. The supervisor reconnects, and
    // the reconnect's HTTP I/O proceeds in real time (paused mode only
    // affects timers, not the I/O reactor).
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(95)).await;
    tokio::time::resume();

    let second = timeout(Duration::from_secs(10), next_item(&mut stream))
        .await
        .expect("second notification must arrive after reconnect")
        .expect("stream must not close")
        .expect("no terminal error");
    assert_eq!(second.sequence, 2);

    drop(stream);
}

#[tokio::test]
async fn state_store_checkpoint_round_trip() {
    use aviso::state::{Checkpoint, ResumeKey, StateStore};
    use serde_json::Value;
    use std::sync::Arc;

    // Step 1. A paced TcpListener server emits exactly three notifications
    // and then holds the connection open without further data. The
    // supervisor commits item 1 before sending item 2, commits item 2
    // before sending item 3, and sets `pending_commit = Some(3)`.
    // Sequence 3 stays uncommitted because the supervisor never sends a
    // fourth item. After the consumer reads all three and drops the
    // stream, the store's checkpoint must be EXACTLY 2; the no-final-
    // flush invariant guarantees this.
    let bodies_step_1 = vec![format!(
        "{}{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        sse_chunk("live-notification", &cloud_event("mars", 2)),
        sse_chunk("live-notification", &cloud_event("mars", 3)),
    )];
    let (url_step_1, _server_step_1) = paced_sse_server(bodies_step_1).await;

    let store: Arc<dyn StateStore> = Arc::new(MemoryStore::new());
    let client = AvisoClient::builder()
        .base_url(&url_step_1)
        .state_store(store.clone())
        .build()
        .unwrap();

    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();
    for expected in 1u64..=3 {
        let item = timeout(Duration::from_secs(5), next_item(&mut stream))
            .await
            .expect("notification should arrive")
            .expect("stream open")
            .expect("no error");
        assert_eq!(item.sequence, expected);
    }
    drop(stream);
    drop(client);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let base_url = url::Url::parse(&url_step_1).unwrap();
    let resume_key = ResumeKey::new(
        &base_url,
        "mars",
        &Value::Object(serde_json::Map::default()),
        None,
    )
    .expect("resume key");
    let checkpoint: Checkpoint = store
        .get(&resume_key)
        .await
        .expect("store get")
        .expect("checkpoint must be set after three sends");
    assert_eq!(
        checkpoint.last_committed_sequence, 2,
        "no-final-flush invariant: after sending items 1, 2, 3 the supervisor commits \
         items 1 and 2 (each before the NEXT send); item 3 stays uncommitted because \
         no fourth send ever happens"
    );

    // Step 2. A second paced server. The wire request body for the new
    // watch must contain `from_id = "3"` (the stored cursor 2 + 1),
    // proving the second process resumes from the persisted checkpoint
    // and re-delivers item 3 per at-least-once semantics.
    let captured_body: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let captured_clone = captured_body.clone();
    let bodies_step_2 = vec![sse_chunk("live-notification", &cloud_event("mars", 3))];
    let (url_step_2, _server_step_2) =
        paced_sse_server_with_capture(bodies_step_2, captured_clone).await;
    let base_url_step_2 = url::Url::parse(&url_step_2).unwrap();
    let resume_key_step_2 = ResumeKey::new(
        &base_url_step_2,
        "mars",
        &Value::Object(serde_json::Map::default()),
        None,
    )
    .expect("resume key step 2");
    store
        .put(&resume_key_step_2, checkpoint.clone())
        .await
        .expect("re-seed store under step-2 base url");

    let client_step_2 = AvisoClient::builder()
        .base_url(&url_step_2)
        .state_store(store.clone())
        .build()
        .unwrap();
    let mut stream_step_2 = client_step_2.watch(WatchRequest::watch("mars")).unwrap();
    let item = timeout(Duration::from_secs(5), next_item(&mut stream_step_2))
        .await
        .expect("re-delivered notification should arrive")
        .expect("stream open")
        .expect("no error");
    assert_eq!(
        item.sequence, 3,
        "supervisor re-delivers item 3 per at-least-once"
    );

    let recorded = captured_body
        .lock()
        .unwrap()
        .clone()
        .expect("body recorded");
    let body_json: serde_json::Value = serde_json::from_str(&recorded).expect("body is JSON");
    assert_eq!(
        body_json.get("from_id").and_then(|v| v.as_str()),
        Some("3"),
        "second process must request from_id=3 (stored cursor 2 + 1)"
    );

    drop(stream_step_2);
    drop(client_step_2);
}

/// `paced_sse_server` variant that captures each connection's request body
/// in the provided `Arc<Mutex<Option<String>>>`. The capture records only
/// the first request body so the test can assert on the second process's
/// wire-level resume parameter.
async fn paced_sse_server_with_capture(
    bodies: Vec<String>,
    captured: Arc<Mutex<Option<String>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        let mut index = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = bodies.get(index).cloned().unwrap_or_default();
            let captured_for_this = captured.clone();
            index += 1;
            tokio::spawn(async move {
                let mut request_buf = Vec::with_capacity(4096);
                let mut chunk = [0u8; 1024];
                loop {
                    match tokio::io::AsyncReadExt::read(&mut socket, &mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            request_buf.extend_from_slice(&chunk[..n]);
                            if request_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let raw = String::from_utf8_lossy(&request_buf).into_owned();
                if let Some(idx) = raw.find("\r\n\r\n") {
                    let body_str = raw[idx + 4..].to_string();
                    let mut g = captured_for_this.lock().unwrap();
                    if g.is_none() {
                        *g = Some(body_str);
                    }
                }
                let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: \
                     chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;
                let chunk_frame = format!("{:X}\r\n{}\r\n", body.len(), body);
                let _ = socket.write_all(chunk_frame.as_bytes()).await;
                let _ = socket.flush().await;
                let mut sink = [0u8; 256];
                loop {
                    match tokio::io::AsyncReadExt::read(&mut socket, &mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
        }
    });
    (url, handle)
}

#[tokio::test]
async fn user_supplied_from_wins_over_stored_checkpoint() {
    use aviso::state::{Checkpoint, ResumeKey, StateStore};
    use serde_json::Value;
    use std::sync::Arc;

    let server = MockServer::start().await;
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let captured_clone = captured.clone();

    let body = sse_chunk("live-notification", &cloud_event("mars", 6));
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |req: &Request| {
            let body_str = String::from_utf8_lossy(&req.body).into_owned();
            *captured_clone.lock().unwrap() = Some(body_str);
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body.clone())
        })
        .mount(&server)
        .await;

    let store: Arc<dyn StateStore> = Arc::new(MemoryStore::new());
    let base_url = url::Url::parse(&server.uri()).unwrap();
    let resume_key = ResumeKey::new(
        &base_url,
        "mars",
        &Value::Object(serde_json::Map::default()),
        None,
    )
    .expect("resume key");
    store
        .put(
            &resume_key,
            Checkpoint::new(99, Some("mars@99".to_string())),
        )
        .await
        .expect("preload checkpoint");

    let client = AvisoClient::builder()
        .base_url(server.uri())
        .state_store(store.clone())
        .build()
        .unwrap();

    let mut stream = client
        .watch(WatchRequest::watch_from(
            "mars",
            ResumeStart::AfterSequence(5),
        ))
        .unwrap();

    let _ = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("notification should arrive")
        .expect("stream must not close")
        .expect("no terminal error");

    drop(stream);

    let recorded = captured.lock().unwrap().clone().expect("body recorded");
    let body_json: serde_json::Value = serde_json::from_str(&recorded).expect("body is JSON");
    let from_id = body_json
        .get("from_id")
        .and_then(|v| v.as_str())
        .expect("from_id present");
    assert_eq!(
        from_id, "6",
        "user-supplied AfterSequence(5) must serialise as from_id=6, ignoring the stored \
         checkpoint at sequence 99"
    );
}

#[tokio::test]
async fn parent_drop_cancels_supervisor_blocked_on_full_channel() {
    // Pin the load-bearing invariant: if a supervisor is parked on
    // `tx.send().await` because the bounded mpsc channel is full,
    // dropping the parent `AvisoClient` must STILL terminate the
    // supervisor. Without the parent-cancel arm in `send_or_cancel`
    // the supervisor would block forever, holding the HTTP connection
    // open and any state-store handles.
    let server = MockServer::start().await;
    let mut body = String::new();
    for n in 1..=300 {
        body.push_str(&sse_chunk("live-notification", &cloud_event("mars", n)));
    }
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

    let _ = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("first notification")
        .expect("stream open")
        .expect("no error");

    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(client);

    let end = timeout(Duration::from_secs(2), async {
        while next_item(&mut stream).await.is_some() {}
    })
    .await;
    assert!(
        end.is_ok(),
        "supervisor must exit on parent-drop within 2s even when the channel is full"
    );
}

#[tokio::test]
async fn parent_drop_cancels_children() {
    let server = MockServer::start().await;
    let mut body = String::new();
    for n in 1..=5 {
        body.push_str(&sse_chunk("live-notification", &cloud_event("mars", n)));
    }
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
    let mut stream_a = client.watch(WatchRequest::watch("mars")).unwrap();
    let mut stream_b = client.watch(WatchRequest::watch("cosmo")).unwrap();

    let _first_a = timeout(Duration::from_secs(5), next_item(&mut stream_a))
        .await
        .expect("stream A first notification")
        .expect("stream A open")
        .expect("no error on stream A");

    drop(client);

    let end_a = timeout(Duration::from_secs(2), async {
        while next_item(&mut stream_a).await.is_some() {}
    })
    .await;
    assert!(end_a.is_ok(), "stream A must close after AvisoClient drop");

    let end_b = timeout(Duration::from_secs(2), async {
        while next_item(&mut stream_b).await.is_some() {}
    })
    .await;
    assert!(end_b.is_ok(), "stream B must close after AvisoClient drop");
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
