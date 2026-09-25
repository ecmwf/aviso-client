// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The connections watches run on.
//!
//! A watch's response is meant to stay open, so the request timeout a caller
//! sets for ordinary requests must not apply to it. These tests hold a watch
//! open longer than the timeout against a local server that counts
//! connections: one connection means the watch was never cut.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap on fixture setup is the standard test diagnostic"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use aviso::AvisoClient;
use aviso::watch::WatchRequest;
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Accepts connections, answers each with an opened SSE stream, and keeps it
/// open without writing more. Returns the base URL and the count of
/// connections accepted so far.
async fn holding_sse_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                // The request is small; one read takes its headers, which is
                // all this server needs before answering.
                let mut request = [0u8; 4096];
                if socket.read(&mut request).await.unwrap_or(0) == 0 {
                    return;
                }
                let opening =
                    "event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                     Transfer-Encoding: chunked\r\n\r\n{:X}\r\n{opening}\r\n",
                    opening.len()
                );
                if socket.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
                // Hold the stream open until the client goes away.
                let mut sink = [0u8; 256];
                while socket.read(&mut sink).await.unwrap_or(0) > 0 {}
            });
        }
    });
    (url, accepted)
}

#[tokio::test]
async fn the_request_timeout_does_not_cut_a_watch() {
    let (url, accepted) = holding_sse_server().await;
    let client = AvisoClient::builder()
        .base_url(&url)
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let mut stream = client.watch(WatchRequest::watch("mars")).unwrap();

    // Three times the timeout. A watch the timeout applied to would be cut
    // and reconnected at least twice in this window.
    let waited = tokio::time::timeout(Duration::from_secs(3), stream.next()).await;
    assert!(
        waited.is_err(),
        "the watch should still be waiting, got {waited:?}"
    );
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "the watch reconnected, so the request timeout cut it"
    );
    stream.close().await;
}

#[tokio::test]
async fn ordinary_requests_still_honour_the_timeout() {
    // A server that accepts and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let client = AvisoClient::builder()
        .base_url(&url)
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();

    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(Duration::from_secs(10), client.schema()).await;
    let result = outcome.expect("schema() should end on its own request timeout");
    assert!(
        result.is_err(),
        "a server that never answers cannot succeed"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the 1 s request timeout did not apply: took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn many_watches_on_one_client_all_open() {
    // Over HTTP/1.1 each watch has its own connection anyway; this checks
    // that leasing connections for watches past one connection's worth
    // neither fails nor loses a watch.
    let (url, accepted) = holding_sse_server().await;
    let client = AvisoClient::builder().base_url(&url).build().unwrap();
    let count = 70;
    let streams: Vec<_> = (0..count)
        .map(|_| client.watch(WatchRequest::watch("mars")).unwrap())
        .collect();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while accepted.load(Ordering::SeqCst) < count && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(accepted.load(Ordering::SeqCst), count);
    for stream in streams {
        stream.close().await;
    }
}
