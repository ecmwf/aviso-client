//! Integration tests for `AvisoClient::watch()` with webhook triggers.
//!
//! Cross-platform: webhook is not gated to Unix (unlike command).
//!
//! - A webhook trigger posts per notification against a wiremock server.
//! - A 5xx-returning receiver exhausts the retry budget, surfacing
//!   `ClientError::TriggerFailed { kind: TriggerKindLabel::Webhook, .. }`.
//! - A 4xx with `fail_fast = true` terminates immediately (single
//!   request despite a non-zero retry budget).
//! - A 4xx with `fail_fast = false` retries through the budget, succeeds
//!   on the final attempt.
//! - URL, header values, and body templates render through to the
//!   wire request.
//! - A short per-trigger timeout against a slow receiver returns
//!   `TriggerError::Timeout(t)` within seconds, not the full delay.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::pin::Pin;
use std::time::Duration;

use aviso::watch::{HttpMethod, Trigger, TriggerError, TriggerKindLabel, WatchRequest};
use aviso::{AvisoClient, ClientError};
use futures_core::Stream;
use serde_json::{Value, json};
use tokio::time::timeout;
use wiremock::matchers::{body_string_contains, header, method, path};
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

async fn take_n<S>(mut stream: S, n: usize) -> (Vec<Result<aviso::Notification, ClientError>>, S)
where
    S: Stream<Item = Result<aviso::Notification, ClientError>> + Unpin,
{
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        match timeout(Duration::from_secs(5), next_item(&mut stream))
            .await
            .expect("notification arrives within 5s")
        {
            Some(item) => out.push(item),
            None => break,
        }
    }
    (out, stream)
}

async fn mount_finite_stream(server: &MockServer, body: String) {
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

fn two_notifications_body() -> String {
    format!(
        "{}{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        sse_chunk("live-notification", &cloud_event("mars", 2)),
        end_of_stream_chunk(),
    )
}

fn one_notification_body() -> String {
    format!(
        "{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        end_of_stream_chunk(),
    )
}

#[tokio::test]
async fn webhook_trigger_posts_to_wiremock_for_each_notification() {
    let hook_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(2)
        .mount(&hook_server)
        .await;
    let aviso_server = MockServer::start().await;
    mount_finite_stream(&aviso_server, two_notifications_body()).await;

    let client = client_for(&aviso_server);
    let url = format!("{}/hook", hook_server.uri());
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::webhook(url)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 2).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 2, "expected 2 notifications");
    for (i, item) in items.iter().enumerate() {
        let n = item.as_ref().expect("notification is Ok");
        assert_eq!(n.sequence, u64::try_from(i + 1).unwrap());
    }
    // expect(2) on the mock causes drop to assert exactly 2 received.
}

#[tokio::test]
async fn webhook_trigger_5xx_retried_through_budget_then_terminates() {
    let hook_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream failure"))
        .mount(&hook_server)
        .await;
    let aviso_server = MockServer::start().await;
    mount_finite_stream(&aviso_server, one_notification_body()).await;

    let client = client_for(&aviso_server);
    let url = format!("{}/hook", hook_server.uri());
    // retries(1): one retry after the initial attempt = 2 attempts total,
    // each returning 500; classifier keeps 5xx retryable; after the budget
    // is exhausted the supervisor surfaces a terminal Webhook error.
    let request = WatchRequest::watch("mars")
        .with_triggers(vec![Trigger::webhook(url).required(true).retries(1)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 1, "expected exactly one item (the failure)");
    match &items[0] {
        Err(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Webhook,
            source: TriggerError::Webhook { status, .. },
        }) => {
            assert_eq!(status.map(|s| s.as_u16()), Some(500));
        }
        other => panic!("expected TriggerFailed for webhook 5xx, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_trigger_4xx_retried_with_fail_fast_false_then_succeeds() {
    // fail_fast(false) opts every error including 4xx into the retry
    // budget. Three 4xx responses then a final 2xx; the dispatcher must
    // send four requests total and the supervisor surfaces a successful
    // notification. expect(4) on the failing mock asserts the retry
    // count; the success mock catches the fourth attempt.
    let hook_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(404).set_body_string("transient"))
        .up_to_n_times(3)
        .expect(3)
        .mount(&hook_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&hook_server)
        .await;
    let aviso_server = MockServer::start().await;
    mount_finite_stream(&aviso_server, one_notification_body()).await;

    let client = client_for(&aviso_server);
    let url = format!("{}/hook", hook_server.uri());
    let request = WatchRequest::watch("mars").with_triggers(vec![
        Trigger::webhook(url)
            .required(true)
            .retries(5)
            .fail_fast(false),
    ]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 1);
    assert!(
        items[0].is_ok(),
        "expected successful notification, got: {:?}",
        items[0]
    );
    // The expect(3) + expect(1) on the wiremock Mocks fire on drop and
    // panic if the counts do not match exactly four total attempts.
}

#[tokio::test]
async fn webhook_trigger_4xx_terminates_immediately_with_fail_fast_true() {
    let hook_server = MockServer::start().await;
    // expect(1): even with retries(5), fail_fast=true means a single 4xx
    // bypasses the retry budget and the dispatcher returns immediately.
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .expect(1)
        .mount(&hook_server)
        .await;
    let aviso_server = MockServer::start().await;
    mount_finite_stream(&aviso_server, one_notification_body()).await;

    let client = client_for(&aviso_server);
    let url = format!("{}/hook", hook_server.uri());
    let request = WatchRequest::watch("mars").with_triggers(vec![
        Trigger::webhook(url)
            .required(true)
            .retries(5)
            .fail_fast(true),
    ]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 1);
    match &items[0] {
        Err(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Webhook,
            source: TriggerError::Webhook { status, .. },
        }) => {
            assert_eq!(status.map(|s| s.as_u16()), Some(404));
        }
        other => panic!("expected TriggerFailed for webhook 4xx, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_trigger_template_substitution_in_url_headers_body() {
    // Use notification-path templates only (no env-var templates):
    // env-var rendering is covered by unit tests in webhook/tests.rs
    // via the render_webhook_parts_with_env seam, which lets the test
    // inject a fake resolver. Integration tests stay env-free to
    // avoid std::env::set_var hazards under cargo test's parallel
    // execution.
    let hook_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook/mars/1"))
        .and(header("X-Notification-Sequence", "1"))
        .and(body_string_contains(r#""seq":1"#))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&hook_server)
        .await;
    let aviso_server = MockServer::start().await;
    mount_finite_stream(&aviso_server, one_notification_body()).await;

    let client = client_for(&aviso_server);
    let url = format!(
        "{}/hook/{{{{ notification.event_type }}}}/{{{{ notification.sequence }}}}",
        hook_server.uri()
    );
    let request = WatchRequest::watch("mars").with_triggers(vec![
        Trigger::webhook(url)
            .method(HttpMethod::Post)
            .header("X-Notification-Sequence", "{{ notification.sequence }}")
            .body_template(r#"{"seq":{{ notification.sequence }}}"#),
    ]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 1);
    assert!(items[0].is_ok(), "got: {:?}", items[0]);
}

#[tokio::test]
async fn webhook_trigger_timeout_terminates_with_typed_error() {
    let hook_server = MockServer::start().await;
    // set_delay(10s) much greater than the per-trigger timeout (200ms);
    // reqwest's Request::timeout fires, dispatch returns Timeout(200ms),
    // and the whole test must complete in single-digit seconds.
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
        .mount(&hook_server)
        .await;
    let aviso_server = MockServer::start().await;
    mount_finite_stream(&aviso_server, one_notification_body()).await;

    let client = client_for(&aviso_server);
    let url = format!("{}/hook", hook_server.uri());
    let request = WatchRequest::watch("mars").with_triggers(vec![
        Trigger::webhook(url)
            .required(true)
            .timeout(Duration::from_millis(200)),
    ]);
    let stream = client.watch(request).unwrap();
    let started = std::time::Instant::now();
    let (items, stream) = take_n(stream, 1).await;
    let elapsed = started.elapsed();
    drop(stream);
    drop(client);

    assert!(
        elapsed < Duration::from_secs(5),
        "timeout must terminate the request quickly; elapsed: {elapsed:?}"
    );
    assert_eq!(items.len(), 1);
    match &items[0] {
        Err(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Webhook,
            source: TriggerError::Timeout(t),
        }) => {
            assert_eq!(*t, Duration::from_millis(200));
        }
        other => panic!("expected TriggerFailed for webhook timeout, got {other:?}"),
    }
}
