// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for `AvisoClient::watch()` with command triggers.
//!
//! Unix-only: the entire file is gated behind `#![cfg(unix)]` because
//! the `Trigger::command` API surface it exercises is itself
//! `#[cfg(unix)]`. On non-Unix targets cargo skips the file and the
//! suite still builds.
//!
//! - A command trigger runs per notification against a tempdir script
//!   that appends the sequence number to a file; after two
//!   notifications the file has two lines and the watch stream
//!   delivered both.
//! - A required command trigger that exits non-zero terminates the
//!   watch with `ClientError::TriggerFailed { kind:
//!   TriggerKindLabel::Command, source: TriggerError::Command { .. } }`.
//! - A command trigger configured with a short per-trigger timeout
//!   against a long-running script returns
//!   `ClientError::TriggerFailed { source: TriggerError::Timeout(..) }`
//!   and terminates within a few seconds (not the full sleep duration).

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::pin::Pin;
use std::time::Duration;

use aviso::watch::{Trigger, TriggerError, TriggerKindLabel, WatchRequest};
use aviso::{AvisoClient, ClientError};
use futures_core::Stream;
use serde_json::{Value, json};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};
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
        .respond_with(move |request: &wiremock::Request| common::opened_sse(request, &body))
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
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(c) = std::fs::read_to_string(path)
                && c.lines().count() >= min_lines
            {
                return c;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("file reaches expected line count within 5s")
}

#[tokio::test]
async fn command_trigger_runs_per_notification_against_tempdir_script() {
    let dir = tempfile::tempdir().unwrap();
    let outfile = dir.path().join("seq.out");
    let outfile_str = outfile.display().to_string();

    let server = MockServer::start().await;
    mount_finite_stream(&server, two_notifications_body()).await;

    let client = client_for(&server);
    let cmd = format!("printf '%s\\n' \"$AVISO_SEQUENCE\" >> '{outfile_str}'");
    let request = WatchRequest::watch("mars").with_triggers(vec![Trigger::command(cmd)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 2).await;
    drop(stream);
    drop(client);

    assert_eq!(items.len(), 2, "expected 2 notifications");
    for (i, item) in items.iter().enumerate() {
        let n = item.as_ref().expect("notification is Ok");
        assert_eq!(n.sequence, u64::try_from(i + 1).unwrap());
    }

    let contents = read_until_lines(&outfile, 2).await;
    let lines: Vec<&str> = contents.lines().collect();
    assert!(
        lines.len() >= 2,
        "expected at least 2 lines, got {contents:?}"
    );
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
}

#[tokio::test]
async fn required_command_trigger_failure_terminates_watch_with_typed_error() {
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
    // exit 7 to verify the exit code surfaces in the error.
    let request =
        WatchRequest::watch("mars").with_triggers(vec![Trigger::command("exit 7").required(true)]);
    let stream = client.watch(request).unwrap();
    let (items, stream) = take_n(stream, 1).await;
    drop(stream);
    drop(client);

    assert_eq!(
        items.len(),
        1,
        "expected exactly one item (the trigger failure)"
    );
    match &items[0] {
        Err(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Command,
            source: TriggerError::Command { exit_code, .. },
        }) => {
            assert_eq!(*exit_code, 7, "got exit_code: {exit_code}");
        }
        other => panic!("expected ClientError::TriggerFailed for command trigger, got {other:?}"),
    }
}

#[tokio::test]
async fn command_trigger_timeout_terminates_via_kill_with_typed_error() {
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
    // sleep 30 must NOT take 30 seconds: the per-trigger timeout
    // kicks in at 200ms, the dispatcher kills the shell, reaps the
    // zombie, and returns TriggerError::Timeout. The whole test must
    // complete in single-digit seconds.
    let request = WatchRequest::watch("mars").with_triggers(vec![
        Trigger::command("sleep 30")
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
        "timeout must terminate the child quickly; elapsed: {elapsed:?}"
    );
    assert_eq!(
        items.len(),
        1,
        "expected exactly one item (the trigger failure)"
    );
    match &items[0] {
        Err(ClientError::TriggerFailed {
            kind: TriggerKindLabel::Command,
            source: TriggerError::Timeout(t),
        }) => {
            assert_eq!(*t, Duration::from_millis(200));
        }
        other => panic!("expected ClientError::TriggerFailed for command timeout, got {other:?}"),
    }
}
