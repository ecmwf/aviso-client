// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Where a reconnect resumes: after the last notification delivered, so a
//! routine server close does not deliver that notification again.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]

use std::time::Duration;

use aviso::AvisoClient;
use aviso::watch::WatchRequest;
use serde_json::{Value, json};
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request};

mod common;

fn sse_chunk(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn notification(sequence: u64) -> String {
    sse_chunk(
        "live-notification",
        &json!({
            "id": format!("mars@{sequence}"),
            "data": {"identifier": {}, "payload": null},
        }),
    )
}

fn max_duration_reached() -> String {
    sse_chunk(
        "connection-closing",
        &json!({"reason": "max_duration_reached", "timestamp": "2026-10-02T00:00:00Z",
                "message": "", "topic": "mars"}),
    )
}

/// The server is asked for `from_id = 4` after 1, 2 and 3, and each
/// notification arrives once.
#[tokio::test]
async fn reconnect_resumes_after_the_last_delivered_notification() {
    let server = MockServer::start().await;
    let first = format!(
        "{}{}{}{}",
        notification(1),
        notification(2),
        notification(3),
        max_duration_reached()
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |request: &Request| common::opened_sse(request, &first))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let second = notification(4);
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |request: &Request| common::opened_sse(request, &second))
        .mount(&server)
        .await;

    let client = AvisoClient::builder()
        .base_url(server.uri())
        .build()
        .unwrap();
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();
    let mut sequences = Vec::new();
    for _ in 0..4 {
        let item = timeout(Duration::from_secs(5), stream.recv())
            .await
            .expect("each notification should arrive within 5s")
            .expect("stream must not close before four items")
            .expect("no terminal error should surface");
        sequences.push(item.sequence);
    }
    timeout(Duration::from_secs(5), stream.close())
        .await
        .expect("the stream should close within 5s");

    assert_eq!(
        sequences,
        [1, 2, 3, 4],
        "no notification is delivered twice"
    );
    let requests = server.received_requests().await.unwrap();
    let reconnect: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(reconnect["from_id"], "4");
}
