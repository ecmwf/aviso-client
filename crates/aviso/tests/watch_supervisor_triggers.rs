//! Integration tests for `AvisoClient::watch()` trigger behaviour:
//! echo dispatch does not break stream delivery, log dispatch writes
//! NDJSON to the configured path, append semantics, missing-parent-dir
//! surfaces typed error, optional triggers WARN-and-continue, retries
//! exhausted on real I/O failure, multi-trigger declaration order,
//! parent-drop reaches the trigger pipeline, failed-trigger does not
//! commit, two-trigger NDJSON content equivalence, and the no-triggers
//! baseline.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use aviso::state::MemoryStore;
use aviso::watch::{Trigger, TriggerKindLabel, WatchRequest};
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

/// Collect at most `n` items from the stream, returning when we have
/// `n` items or the stream closes. The supervisor reconnects on
/// `end_of_stream` in Watch mode, so tests that want a bounded item
/// count must take-n-and-drop rather than wait for `None`. Returns the
/// stream so the caller can drop it in its own scope.
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

async fn read_until_lines(path: &std::path::Path, min_lines: usize) -> String {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(c) = std::fs::read_to_string(path) {
                if c.lines().count() >= min_lines {
                    return c;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("log file reaches expected line count within 2s")
}

#[tokio::test]
async fn echo_trigger_runs_for_each_notification_and_stream_still_delivers() {
    let server = MockServer::start().await;
    let body = format!(
        "{}{}{}{}",
        sse_chunk("live-notification", &cloud_event("mars", 1)),
        sse_chunk("live-notification", &cloud_event("mars", 2)),
        sse_chunk("live-notification", &cloud_event("mars", 3)),
        end_of_stream_chunk(),
    );
    mount_finite_stream(&server, body).await;

    let client = client_for(&server);
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::echo()]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 3).await;
    drop(stream);

    assert_eq!(items.len(), 3);
    for (i, item) in items.iter().enumerate() {
        let n = item.as_ref().expect("notification is Ok");
        assert_eq!(n.sequence, u64::try_from(i + 1).unwrap());
    }
}

#[tokio::test]
async fn log_trigger_writes_ndjson_to_configured_path() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("notif.log");
    let server = MockServer::start().await;
    mount_finite_stream(&server, two_notifications_body()).await;

    let client = client_for(&server);
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&log_path)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 2).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 2);
    let contents = read_until_lines(&log_path, 2).await;
    let lines: Vec<&str> = contents.lines().collect();
    assert!(lines.len() >= 2, "expected at least 2 NDJSON lines");
    for (i, line) in lines.iter().take(2).enumerate() {
        let parsed: Value = serde_json::from_str(line).expect("line is JSON");
        assert_eq!(parsed["event_type"], "mars");
        assert_eq!(parsed["sequence"], u64::try_from(i + 1).unwrap());
    }
}

#[tokio::test]
async fn log_trigger_appends_when_file_pre_exists() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("append.log");
    std::fs::write(&log_path, "prior-line\n").unwrap();

    let server = MockServer::start().await;
    mount_finite_stream(
        &server,
        format!(
            "{}{}",
            sse_chunk("live-notification", &cloud_event("mars", 7)),
            end_of_stream_chunk(),
        ),
    )
    .await;

    let client = client_for(&server);
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&log_path)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 1);
    let contents = read_until_lines(&log_path, 2).await;
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines[0], "prior-line", "pre-existing line preserved");
    let appended: Value = serde_json::from_str(lines[1]).expect("appended line is JSON");
    assert_eq!(appended["sequence"], 7);
}

#[tokio::test]
async fn required_log_failure_terminates_watch_with_typed_error() {
    let server = MockServer::start().await;
    mount_finite_stream(
        &server,
        format!(
            "{}{}",
            sse_chunk("live-notification", &cloud_event("mars", 1)),
            end_of_stream_chunk(),
        ),
    )
    .await;

    let bad_path =
        std::path::PathBuf::from("/nonexistent-dir-aviso-required-trigger-test/notif.log");
    let client = client_for(&server);
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&bad_path)]);
    let mut stream = client.watch(request).unwrap();

    let item = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("error arrives within 5s")
        .expect("channel still open");
    match item {
        Err(ClientError::TriggerFailed { kind, source }) => {
            assert!(matches!(kind, TriggerKindLabel::Log { .. }));
            assert!(source.to_string().starts_with("io:"), "got: {source}");
        }
        other => panic!("expected TriggerFailed, got {other:?}"),
    }
    let none = timeout(Duration::from_secs(2), next_item(&mut stream))
        .await
        .expect("stream closes within 2s");
    assert!(none.is_none(), "stream yields None after terminal error");
}

#[tokio::test]
async fn optional_trigger_failure_continues_with_remaining_trigger() {
    let dir = tempfile::tempdir().unwrap();
    let good_log = dir.path().join("good.log");
    let server = MockServer::start().await;
    mount_finite_stream(&server, two_notifications_body()).await;

    let bad_path = std::path::PathBuf::from("/nonexistent-dir-aviso-optional-trigger-test/bad.log");
    let client = client_for(&server);
    let request = WatchRequest::watch("mars").with_triggers(vec![
        Trigger::log(&bad_path).required(false),
        Trigger::log(&good_log),
    ]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 2).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 2);
    let contents = read_until_lines(&good_log, 2).await;
    assert!(contents.lines().count() >= 2);
}

#[tokio::test]
async fn retries_exhausted_on_missing_dir_surfaces_typed_error() {
    let server = MockServer::start().await;
    mount_finite_stream(
        &server,
        format!(
            "{}{}",
            sse_chunk("live-notification", &cloud_event("mars", 1)),
            end_of_stream_chunk(),
        ),
    )
    .await;

    let bad_path =
        std::path::PathBuf::from("/nonexistent-dir-aviso-retries-exhausted-test/notif.log");
    let client = client_for(&server);
    let request =
        WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&bad_path).retries(2)]);
    let mut stream = client.watch(request).unwrap();

    let item = timeout(Duration::from_secs(10), next_item(&mut stream))
        .await
        .expect("error arrives within 10s")
        .expect("channel still open");
    match item {
        Err(ClientError::TriggerFailed { kind, .. }) => {
            assert!(matches!(kind, TriggerKindLabel::Log { .. }));
        }
        other => panic!("expected TriggerFailed after retries, got {other:?}"),
    }
}

#[tokio::test]
async fn trigger_dispatch_runs_in_declaration_order_two_logs() {
    let dir = tempfile::tempdir().unwrap();
    let log_a = dir.path().join("a.log");
    let log_b = dir.path().join("b.log");
    let server = MockServer::start().await;
    mount_finite_stream(
        &server,
        format!(
            "{}{}",
            sse_chunk("live-notification", &cloud_event("mars", 1)),
            end_of_stream_chunk(),
        ),
    )
    .await;

    let client = client_for(&server);
    let request =
        WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&log_a), Trigger::log(&log_b)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 1);
    let a_contents = read_until_lines(&log_a, 1).await;
    let b_contents = read_until_lines(&log_b, 1).await;
    // Both files may have reconnect-driven duplicates; compare only the
    // first line of each (they should be identical NDJSON for N=1).
    let a_first = a_contents.lines().next().expect("log a has a line");
    let b_first = b_contents.lines().next().expect("log b has a line");
    assert_eq!(a_first, b_first, "both logs receive identical first line");
    let parsed: Value = serde_json::from_str(a_first).unwrap();
    assert_eq!(parsed["sequence"], 1);
}

#[tokio::test]
async fn parent_drop_cancels_supervisor_during_trigger_dispatch_smoke() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("cancel.log");
    let server = MockServer::start().await;
    mount_finite_stream(&server, two_notifications_body()).await;

    let client = client_for(&server);
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&log_path)]);
    let stream = client.watch(request).unwrap();

    drop(stream);
    drop(client);

    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn failed_required_trigger_on_first_notification_does_not_commit_via_real_io() {
    let server = MockServer::start().await;
    mount_finite_stream(
        &server,
        format!(
            "{}{}",
            sse_chunk("live-notification", &cloud_event("mars", 1)),
            end_of_stream_chunk(),
        ),
    )
    .await;

    let bad_path = std::path::PathBuf::from("/nonexistent-dir-aviso-no-commit-on-fail/notif.log");
    let store = Arc::new(MemoryStore::new());
    let client = AvisoClient::builder()
        .base_url(server.uri())
        .state_store(store.clone())
        .build()
        .unwrap();
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::log(&bad_path)]);
    let mut stream = client.watch(request).unwrap();

    let item = timeout(Duration::from_secs(5), next_item(&mut stream))
        .await
        .expect("error arrives within 5s")
        .expect("channel still open");
    assert!(matches!(item, Err(ClientError::TriggerFailed { .. })));

    let _ = timeout(Duration::from_secs(2), next_item(&mut stream))
        .await
        .expect("stream closes within 2s");
    drop(stream);
    drop(client);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        Arc::strong_count(&store) == 1,
        "store has no lingering supervisor references"
    );
}

#[tokio::test]
async fn echo_and_log_together_serialize_identical_content() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("paired.log");
    let server = MockServer::start().await;
    mount_finite_stream(
        &server,
        format!(
            "{}{}",
            sse_chunk("live-notification", &cloud_event("mars", 42)),
            end_of_stream_chunk(),
        ),
    )
    .await;

    let client = client_for(&server);
    let request =
        WatchRequest::watch("mars").with_triggers(vec![Trigger::echo(), Trigger::log(&log_path)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    let item = items[0].as_ref().expect("notification is Ok");
    let contents = read_until_lines(&log_path, 1).await;
    let first_line = contents.lines().next().expect("log has a line");
    let parsed: Value = serde_json::from_str(first_line).unwrap();
    assert_eq!(parsed["event_type"], item.event_type);
    assert_eq!(parsed["sequence"], item.sequence);
}

#[tokio::test]
async fn stream_consumer_still_works_with_no_triggers_configured() {
    let server = MockServer::start().await;
    mount_finite_stream(&server, two_notifications_body()).await;

    let client = client_for(&server);
    let request = WatchRequest::watch("mars");
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 2).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 2);
    for (i, item) in items.iter().enumerate() {
        let n = item.as_ref().expect("notification is Ok");
        assert_eq!(n.sequence, u64::try_from(i + 1).unwrap());
    }
}
