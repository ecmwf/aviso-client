// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `AvisoClient::watch_many`: several watches read through one stream.
//!
//! A mock server answers each watch according to its event type, so each
//! test controls what every named watch sees.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use aviso::state::{Checkpoint, MemoryStore, ResumeKey, StateStore, StoreError};

use aviso::watch::{ErrorPolicy, MultiNotificationStream, ResumeStart, WatchRequest};
use aviso::{AvisoClient, ClientError};
use futures_util::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

fn sse(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn notification(event_type: &str, sequence: u64) -> String {
    sse(
        "replay",
        &json!({
            "id": format!("{event_type}@{sequence}"),
            "source": "https://aviso.example",
            "type": format!("int.ecmwf.aviso.{event_type}"),
            "time": "2026-05-17T12:34:56Z",
            "data": { "identifier": {}, "payload": null }
        }),
    )
}

/// A replay that delivers `count` notifications, completes, and closes.
fn finished_replay(event_type: &str, count: u64) -> String {
    let mut body: String = (1..=count).map(|n| notification(event_type, n)).collect();
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

async fn serve(server: &MockServer, event_type: &str, body: String) {
    // Replay-only watches call /api/v1/replay, others /api/v1/watch.
    Mock::given(method("POST"))
        .and(path_regex("^/api/v1/(watch|replay)$"))
        .and(body_partial_json(json!({ "event_type": event_type })))
        .respond_with(move |request: &wiremock::Request| common::opened_sse(request, &body))
        .mount(server)
        .await;
}

async fn refuse(server: &MockServer, event_type: &str) {
    // Replay-only watches call /api/v1/replay, others /api/v1/watch.
    Mock::given(method("POST"))
        .and(path_regex("^/api/v1/(watch|replay)$"))
        .and(body_partial_json(json!({ "event_type": event_type })))
        .respond_with(ResponseTemplate::new(400).set_body_string("unknown field 'stepp'"))
        .mount(server)
        .await;
}

fn replay(event_type: &str) -> WatchRequest {
    WatchRequest::replay_only(event_type, ResumeStart::AfterSequence(0))
}

fn client(server: &MockServer) -> AvisoClient {
    AvisoClient::builder()
        .base_url(server.uri())
        .build()
        .unwrap()
}

/// One merged item, reduced to what the tests compare: the watch's name with
/// either the notification's sequence or the error.
type Item = Result<(String, u64), (String, ClientError)>;

/// Reads to the end, with a bound so a stream that never ends fails the test.
async fn drain(mut stream: MultiNotificationStream) -> Vec<Item> {
    let mut out = Vec::new();
    loop {
        let item = tokio::time::timeout(Duration::from_secs(60), stream.next())
            .await
            .expect("the merged stream should end on its own");
        match item {
            Some(Ok((name, n))) => out.push(Ok((name, n.sequence))),
            Some(Err(e)) => out.push(Err((e.name, e.error))),
            None => return out,
        }
    }
}

fn sequences_of(items: &[Item], name: &str) -> Vec<u64> {
    items
        .iter()
        .filter_map(|i| match i {
            Ok((n, s)) if n == name => Some(*s),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn each_notification_carries_its_watch_name_and_the_stream_ends_when_all_do() {
    let server = MockServer::start().await;
    serve(&server, "mars", finished_replay("mars", 3)).await;
    serve(&server, "wave", finished_replay("wave", 2)).await;

    let client = client(&server);
    let stream = client
        .watch_many(
            [("surface", replay("mars")), ("waves", replay("wave"))],
            ErrorPolicy::Stop,
        )
        .unwrap();
    let items = drain(stream).await;

    assert_eq!(sequences_of(&items, "surface"), vec![1, 2, 3]);
    assert_eq!(sequences_of(&items, "waves"), vec![1, 2]);
    assert_eq!(items.len(), 5, "no errors expected: {items:?}");
}

#[tokio::test]
async fn continue_drops_the_failed_watch_and_keeps_the_others() {
    let server = MockServer::start().await;
    refuse(&server, "broken").await;
    serve(&server, "mars", finished_replay("mars", 3)).await;

    let client = client(&server);
    let stream = client
        .watch_many(
            [("bad", replay("broken")), ("good", replay("mars"))],
            ErrorPolicy::Continue,
        )
        .unwrap();
    let items = drain(stream).await;

    let errors: Vec<_> = items.iter().filter_map(|i| i.as_ref().err()).collect();
    assert_eq!(errors.len(), 1, "{items:?}");
    assert_eq!(errors[0].0, "bad");
    assert!(
        matches!(errors[0].1, ClientError::Http { status: 400, .. }),
        "{:?}",
        errors[0].1
    );
    assert_eq!(sequences_of(&items, "good"), vec![1, 2, 3]);
}

#[tokio::test]
async fn stop_ends_everything_after_the_first_error() {
    let server = MockServer::start().await;
    refuse(&server, "broken").await;
    // A live watch that would run forever: open, then nothing.
    serve(&server, "mars", String::new()).await;

    let client = client(&server);
    let mut stream = client
        .watch_many(
            [
                ("bad", replay("broken")),
                ("forever", WatchRequest::watch("mars")),
            ],
            ErrorPolicy::Stop,
        )
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .unwrap();
    let error = first
        .expect("an item")
        .expect_err("the refused watch fails");
    assert_eq!(error.name, "bad");
    assert!(error.to_string().starts_with("watch 'bad': "), "{error}");
    assert!(
        tokio::time::timeout(Duration::from_secs(60), stream.next())
            .await
            .unwrap()
            .is_none(),
        "the other watch should have been closed"
    );
    assert!(stream.running().is_empty(), "{:?}", stream.running());
    // close() waits for every watch, the stopped ones included.
    tokio::time::timeout(Duration::from_secs(60), stream.close())
        .await
        .expect("close() should finish once the watches have stopped");
}

#[tokio::test]
async fn running_lists_the_watches_still_producing() {
    let server = MockServer::start().await;
    refuse(&server, "broken").await;
    serve(&server, "mars", String::new()).await;
    let client = client(&server);
    let mut stream = client
        .watch_many(
            [
                ("bad", replay("broken")),
                ("forever", WatchRequest::watch("mars")),
            ],
            ErrorPolicy::Continue,
        )
        .unwrap();
    assert_eq!(stream.running(), vec!["bad", "forever"]);
    let error = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .unwrap()
        .expect("an item")
        .expect_err("the refused watch fails");
    assert_eq!(error.name, "bad");
    assert_eq!(stream.running(), vec!["forever"]);
    tokio::time::timeout(Duration::from_secs(60), stream.close())
        .await
        .expect("close() should finish");
}

#[tokio::test]
async fn bad_input_is_refused_before_any_watch_opens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let client = client(&server);

    let none: [(&str, WatchRequest); 0] = [];
    let cases: Vec<(Vec<(&str, WatchRequest)>, &str)> = vec![
        (none.into_iter().collect(), "at least one request"),
        (vec![("", replay("mars"))], "must not be empty"),
        (
            vec![("same", replay("mars")), ("same", replay("wave"))],
            "used twice",
        ),
        (
            vec![
                ("fine", replay("mars")),
                (
                    "broken",
                    WatchRequest::replay_only("mars", ResumeStart::AfterSequence(u64::MAX)),
                ),
            ],
            "watch 'broken'",
        ),
    ];
    for (requests, expected) in cases {
        let error = client
            .watch_many(requests, ErrorPolicy::Stop)
            .expect_err("bad input must be refused");
        assert!(
            matches!(&error, ClientError::Config(m) if m.contains(expected)),
            "expected '{expected}' in {error:?}"
        );
    }
    // Give a wrongly opened watch time to reach the server.
    tokio::time::sleep(Duration::from_millis(200)).await;
    server.verify().await;
}

#[tokio::test]
async fn each_watch_keeps_its_own_filter() {
    let server = MockServer::start().await;
    serve(&server, "mars", finished_replay("mars", 1)).await;
    let oper: BTreeMap<String, Value> = [("stream".to_string(), json!("oper"))].into();
    let wave: BTreeMap<String, Value> = [("stream".to_string(), json!("wave"))].into();

    let client = client(&server);
    let stream = client
        .watch_many(
            [
                ("oper", replay("mars").with_filter(oper)),
                ("wave", replay("mars").with_filter(wave)),
            ],
            ErrorPolicy::Stop,
        )
        .unwrap();
    drain(stream).await;

    let mut filters: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.body_json::<Value>().unwrap()["identifier"]["stream"].clone())
        .collect();
    filters.sort_by_key(ToString::to_string);
    assert_eq!(filters, vec![json!("oper"), json!("wave")]);
}

/// A store whose writes take a while, recording the last sequence written,
/// so a test can tell whether `close()` waited for a watch's final write.
struct SlowStore {
    inner: MemoryStore,
    last_written: Mutex<Option<u64>>,
}

#[async_trait]
impl StateStore for SlowStore {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError> {
        self.inner.get(key).await
    }

    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let sequence = checkpoint.last_committed_sequence;
        self.inner.put(key, checkpoint).await?;
        *self.last_written.lock().unwrap() = Some(sequence);
        Ok(())
    }

    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError> {
        self.inner.delete(key).await
    }
}

/// Reads one HTTP request and returns its JSON body.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Value {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0, "connection closed before the request was read");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
    let length: usize = headers
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .map_or(0, |v| v.trim().parse().unwrap());
    while buf.len() < header_end + length {
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0, "connection closed before the body was read");
        buf.extend_from_slice(&chunk[..n]);
    }
    serde_json::from_slice(&buf[header_end..header_end + length]).unwrap()
}

/// Serves a live "mars" watch that delivers mars@1 on its first connection
/// and nothing afterwards, and refuses a "broken" watch with 400, but only
/// once `release` is notified. The test decides when the failure happens,
/// so nothing depends on timing.
async fn gated_server(release: Arc<tokio::sync::Notify>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let first_mars = Arc::new(std::sync::atomic::AtomicBool::new(true));
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let release = Arc::clone(&release);
            let first_mars = Arc::clone(&first_mars);
            tokio::spawn(async move {
                let body = read_request(&mut socket).await;
                if body["event_type"] == "broken" {
                    release.notified().await;
                    let refusal = "HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n";
                    socket.write_all(refusal.as_bytes()).await.unwrap();
                    return;
                }
                let mut events = sse(
                    "live-notification",
                    &json!({"type": "connection_established"}),
                );
                if first_mars.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    events.push_str(&sse(
                        "live-notification",
                        &json!({
                            "id": "mars@1",
                            "source": "https://aviso.example",
                            "type": "int.ecmwf.aviso.mars",
                            "time": "2026-05-17T12:34:56Z",
                            "data": { "identifier": {}, "payload": null }
                        }),
                    ));
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                     transfer-encoding: chunked\r\n\r\n{:X}\r\n{events}\r\n",
                    events.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                // Hold the stream open until the client goes away.
                let mut sink = [0u8; 256];
                while socket.read(&mut sink).await.unwrap_or(0) > 0 {}
            });
        }
    });
    url
}

#[tokio::test]
async fn close_waits_for_the_final_checkpoint_of_a_stopped_watch() {
    // The live watch's one notification is received before the other watch
    // is allowed to fail, so its cursor is pending (unwritten) when the
    // failure stops it, and only the exit flush writes it.
    let release = Arc::new(tokio::sync::Notify::new());
    let url = gated_server(Arc::clone(&release)).await;
    let store = Arc::new(SlowStore {
        inner: MemoryStore::new(),
        last_written: Mutex::new(None),
    });
    let client = AvisoClient::builder()
        .base_url(&url)
        .state_store(store.clone())
        .flush_cursor_on_exit(true)
        .build()
        .unwrap();
    let mut stream = client
        .watch_many(
            [
                ("live", WatchRequest::watch("mars")),
                ("bad", replay("broken")),
            ],
            ErrorPolicy::Stop,
        )
        .unwrap();

    let (name, notification) = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .unwrap()
        .expect("an item")
        .expect("the live notification");
    assert_eq!((name.as_str(), notification.sequence), ("live", 1));
    release.notify_one();
    let error = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .unwrap()
        .expect("an item")
        .expect_err("the refused watch fails");
    assert_eq!(error.name, "bad");
    stream.close().await;
    // close() returns only after the stopped watch has written its cursor.
    assert_eq!(*store.last_written.lock().unwrap(), Some(1));
}
