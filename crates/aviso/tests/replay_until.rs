// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! A replay with an end point across a dropped connection.
//!
//! The mock server answers like aviso-server 0.13.0: `replay_started` reports
//! the resolved `end_sequence`, and a request whose start is past its end is
//! refused with 400. Each first connection ends without `replay_completed`, as
//! a dropped connection does, so the client has to decide whether to
//! reconnect and with which body.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aviso::AvisoClient;
use aviso::watch::{NotificationStream, ReplayEnd, ResumeStart, WatchRequest};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn replay_started(end_sequence: u64) -> String {
    sse(
        "replay-control",
        &json!({"type": "replay_started", "topic": "mars", "end_sequence": end_sequence}),
    )
}

fn notification(sequence: u64) -> String {
    sse(
        "replay",
        &json!({
            "id": format!("mars@{sequence}"),
            "source": "https://aviso.example",
            "type": "int.ecmwf.aviso.mars",
            "time": "2026-09-01T00:00:00Z",
            "data": {"identifier": {}, "payload": null}
        }),
    )
}

fn completed() -> String {
    format!(
        "{}{}",
        sse(
            "replay-control",
            &json!({"type": "replay_completed", "topic": "mars",
                    "timestamp": "2026-09-01T00:00:00Z"}),
        ),
        sse(
            "connection-closing",
            &json!({"reason": "end_of_stream", "timestamp": "2026-09-01T00:00:00Z",
                    "message": "done", "topic": "mars", "request_id": "r"}),
        )
    )
}

fn stream_body(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

/// Answers a replay request whose body contains `matching`.
async fn answer(server: &MockServer, matching: Value, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/api/v1/replay"))
        .and(body_partial_json(matching))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Reads the stream to its end, with a bound so a stream that never ends
/// fails the test instead of hanging it.
async fn drain(mut stream: NotificationStream) -> Vec<u64> {
    let mut sequences = Vec::new();
    loop {
        let item = tokio::time::timeout(Duration::from_secs(60), stream.recv())
            .await
            .expect("the replay should end on its own");
        match item {
            Some(Ok(notification)) => sequences.push(notification.sequence),
            Some(Err(error)) => panic!("the replay should end without an error: {error}"),
            None => return sequences,
        }
    }
}

async fn replay_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.body_json().unwrap())
        .collect()
}

fn client(server: &MockServer) -> AvisoClient {
    AvisoClient::builder()
        .base_url(server.uri())
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_drop_after_the_end_ends_the_replay_without_reconnecting() {
    let server = MockServer::start().await;
    // A date start whose first notification is already the end: nothing is
    // committed yet, so a reconnect would ask for a start past the end.
    answer(
        &server,
        json!({"from_date": "2026-09-01T00:00:00Z", "to_id": "1"}),
        stream_body(format!("{}{}", replay_started(1), notification(1))),
    )
    .await;
    // What aviso-server answers to a start past the end.
    answer(
        &server,
        json!({"from_id": "2"}),
        ResponseTemplate::new(400).set_body_string("to_id (1) must not be lower than from_id (2)"),
    )
    .await;

    let request = WatchRequest::replay_range(
        "mars",
        ResumeStart::Date("2026-09-01T00:00:00Z".into()),
        ReplayEnd::Sequence(1),
    );
    // The client must outlive the stream: dropping it cancels its watches.
    let client = client(&server);
    let stream = client.watch(request).unwrap();

    assert_eq!(drain(stream).await, [1]);
    assert_eq!(replay_bodies(&server).await.len(), 1, "no second request");
}

/// Collects every log line written while it is the default subscriber.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_finished_replay_ends_without_a_retry() {
    let server = MockServer::start().await;
    answer(
        &server,
        json!({"from_id": "1", "to_id": "1"}),
        stream_body(format!("{}{}", replay_started(1), notification(1))),
    )
    .await;
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::INFO)
        .finish();
    // The current-thread runtime runs the supervisor on this thread, so the
    // thread's default subscriber sees its events.
    let _default = tracing::subscriber::set_default(subscriber);

    let request = WatchRequest::replay_range(
        "mars",
        ResumeStart::AfterSequence(0),
        ReplayEnd::Sequence(1),
    );
    let client = client(&server);
    let stream = client.watch(request).unwrap();
    assert_eq!(drain(stream).await, [1]);

    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains("Retrying listener connection"), "{logs}");
}

#[tokio::test]
async fn a_committed_replay_that_delivered_its_end_does_not_reconnect() {
    let server = MockServer::start().await;
    // Sequence 1 is committed and 2, the end, is delivered but pending.
    answer(
        &server,
        json!({"from_id": "1", "to_id": "2"}),
        stream_body(format!(
            "{}{}{}",
            replay_started(2),
            notification(1),
            notification(2)
        )),
    )
    .await;

    let request = WatchRequest::replay_range(
        "mars",
        ResumeStart::AfterSequence(0),
        ReplayEnd::Sequence(2),
    );
    let client = client(&server);
    let stream = client.watch(request).unwrap();

    assert_eq!(drain(stream).await, [1, 2]);
    assert_eq!(replay_bodies(&server).await.len(), 1, "no second request");
}

#[tokio::test]
async fn a_date_end_is_resolved_once_and_sent_as_to_id_on_reconnect() {
    let server = MockServer::start().await;
    // The first connection resolves the date to sequence 3 and drops after
    // the first notification.
    answer(
        &server,
        json!({"from_date": "2026-09-01T00:00:00Z", "to_date": "2026-09-02T00:00:00Z"}),
        stream_body(format!("{}{}", replay_started(3), notification(1))),
    )
    .await;
    // The reconnect carries the resolved end, so the replay cannot grow.
    answer(
        &server,
        json!({"from_id": "2", "to_id": "3"}),
        stream_body(format!(
            "{}{}{}{}",
            replay_started(3),
            notification(2),
            notification(3),
            completed()
        )),
    )
    .await;

    let request = WatchRequest::replay_range(
        "mars",
        ResumeStart::Date("2026-09-01T00:00:00Z".into()),
        ReplayEnd::Date("2026-09-02T00:00:00Z".into()),
    );
    let client = client(&server);
    let stream = client.watch(request).unwrap();

    assert_eq!(drain(stream).await, [1, 2, 3]);
    let bodies = replay_bodies(&server).await;
    assert_eq!(bodies.len(), 2, "{bodies:?}");
    assert_eq!(bodies[1]["to_id"], "3");
    assert!(bodies[1].get("to_date").is_none(), "{}", bodies[1]);
}

#[tokio::test]
async fn a_drop_before_the_end_resumes_up_to_the_same_end() {
    let server = MockServer::start().await;
    // As for any replay, the reconnect starts after the committed sequence,
    // so the last delivered notification arrives again. Both requests have
    // the same body: the first gets the dropped connection, the second the
    // rest of the replay.
    Mock::given(method("POST"))
        .and(path("/api/v1/replay"))
        .and(body_partial_json(json!({"from_id": "1", "to_id": "3"})))
        .respond_with(stream_body(format!(
            "{}{}",
            replay_started(3),
            notification(1)
        )))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    answer(
        &server,
        json!({"from_id": "1", "to_id": "3"}),
        stream_body(format!(
            "{}{}{}{}{}",
            replay_started(3),
            notification(1),
            notification(2),
            notification(3),
            completed()
        )),
    )
    .await;

    let request = WatchRequest::replay_range(
        "mars",
        ResumeStart::AfterSequence(0),
        ReplayEnd::Sequence(3),
    );
    let client = client(&server);
    let stream = client.watch(request).unwrap();

    assert_eq!(drain(stream).await, [1, 1, 2, 3]);
    let bodies = replay_bodies(&server).await;
    assert_eq!(bodies.len(), 2, "{bodies:?}");
    assert!(bodies.iter().all(|body| body["to_id"] == "3"), "{bodies:?}");
}

#[tokio::test]
async fn a_malformed_end_sequence_is_a_protocol_error() {
    let server = MockServer::start().await;
    let started = sse(
        "replay-control",
        &json!({"type": "replay_started", "topic": "mars", "end_sequence": "two"}),
    );
    answer(
        &server,
        json!({"from_id": "1", "to_id": "2"}),
        stream_body(format!("{started}{}", notification(1))),
    )
    .await;

    let request = WatchRequest::replay_range(
        "mars",
        ResumeStart::AfterSequence(0),
        ReplayEnd::Sequence(2),
    );
    let client = client(&server);
    let mut stream = client.watch(request).unwrap();
    let item = tokio::time::timeout(Duration::from_secs(60), stream.recv())
        .await
        .expect("the watch should end on its own");
    match item {
        Some(Err(aviso::ClientError::StreamProtocol { message, .. })) => {
            assert!(message.contains("invalid end_sequence"), "{message}");
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}
