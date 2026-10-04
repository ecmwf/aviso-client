// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The opening deadline on a reconnect of a confirmed watch.
//!
//! Before the first confirmation, an expired opening deadline ends the watch
//! (see `unconfirmed_openings_have_a_fixed_deadline` in `watch_startup.rs`).
//! Once the watch has been confirmed, a reconnect that misses it is a lost
//! connection and is retried. Each test serves three connections from a raw
//! socket: the first confirms the watch and delivers 1, the second stalls,
//! and the third delivers 2.

use std::time::Duration;

use aviso::AvisoClient;
use aviso::watch::WatchRequest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A bound on every wait, so a regression fails the test instead of
/// hanging it.
const WAIT: Duration = Duration::from_secs(10);

/// A response that opens the stream, delivers `sequence`, then closes the
/// connection the way the server does at its maximum duration. A reconnect
/// after a delivery resumes from a sequence, so the server opens it as a
/// replay.
fn delivering(sequence: u64, resumed: bool) -> String {
    let opening = if resumed {
        "event: replay-control\ndata: {\"type\":\"replay_started\"}\n\n"
    } else {
        "event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n"
    };
    let notification = serde_json::json!({
        "id": format!("mars@{sequence}"),
        "data": {"identifier": {}, "payload": null},
    });
    let body = format!(
        "{opening}event: live-notification\ndata: {notification}\n\n\
         event: connection-closing\ndata: {{\"reason\":\"max_duration_reached\",\
         \"timestamp\":\"2026-10-02T00:00:00Z\",\"message\":\"\",\"topic\":\"mars\"}}\n\n"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Serves the three connections; `stalled` is written on the second, which
/// is then held open. `held` fires once it has been written.
async fn serve(
    stalled: &'static str,
) -> Result<
    (
        String,
        tokio::sync::oneshot::Receiver<()>,
        tokio::task::JoinHandle<std::io::Result<()>>,
    ),
    std::io::Error,
> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let (held, held_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut request = [0; 4096];
        let (mut first, _) = listener.accept().await?;
        assert!(first.read(&mut request).await? > 0);
        first.write_all(delivering(1, false).as_bytes()).await?;
        first.shutdown().await?;
        let (mut second, _) = listener.accept().await?;
        assert!(second.read(&mut request).await? > 0);
        second.write_all(stalled.as_bytes()).await?;
        held.send(()).ok(); // The test may already have cancelled.
        let (mut third, _) = listener.accept().await?;
        assert!(third.read(&mut request).await? > 0);
        third.write_all(delivering(2, true).as_bytes()).await?;
        third.shutdown().await?;
        drop(second);
        Ok(())
    });
    Ok((url, held_rx, task))
}

/// Runs one case: the watch delivers 1, the reconnect stalls past the
/// deadline, and the watch still delivers 2.
async fn survives(stalled: &'static str) -> TestResult {
    let (url, held, task) = serve(stalled).await?;
    let client = AvisoClient::builder().base_url(url).build()?;
    let mut stream = client.watch(WatchRequest::watch("mars"))?;

    let first = tokio::time::timeout(WAIT, stream.recv()).await?;
    assert!(
        matches!(first, Some(Ok(ref n)) if n.sequence == 1),
        "{first:?}"
    );
    tokio::time::timeout(WAIT, held).await??;
    // Give the stalled bytes time to reach the client before its clock jumps;
    // the stream must stay quiet meanwhile.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), stream.recv())
            .await
            .is_err()
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();

    let second = tokio::time::timeout(WAIT, stream.recv()).await?;
    assert!(
        matches!(second, Some(Ok(ref n)) if n.sequence == 2),
        "{second:?}"
    );
    tokio::time::timeout(WAIT, stream.close()).await?;
    tokio::time::timeout(WAIT, task).await???;
    Ok(())
}

#[tokio::test]
async fn a_reconnect_that_gets_no_response_is_retried() -> TestResult {
    survives("").await
}

#[tokio::test]
async fn a_reconnect_without_an_opening_event_is_retried() -> TestResult {
    survives(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n\
         1b\r\nevent: heartbeat\ndata: {}\n\n\r\n",
    )
    .await
}
