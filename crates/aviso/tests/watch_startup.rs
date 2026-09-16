// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Public startup validation and deadline regression tests.

use std::time::Duration;

use aviso::watch::{ResumeStart, WatchRequest};
use aviso::{AvisoClient, ClientError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const LIVE: &str = "event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n";
const REPLAY: &str = "event: replay-control\ndata: {\"type\":\"replay_started\"}\n\n";

#[test]
fn invalid_urls_fail_without_exposing_credentials() {
    for url in [
        "ftp://user:SECRET@example.org/private?token=SECRET",
        "not a URL SECRET",
    ] {
        let result = AvisoClient::builder().base_url(url).build();
        assert!(matches!(&result, Err(ClientError::Config(_))));
        assert!(!format!("{result:?}").contains("SECRET"));
    }
}

#[tokio::test]
async fn wrong_content_types_fail_once_before_body_decode() -> TestResult {
    for content_type in [
        None,
        Some("text/html"),
        Some("application/json"),
        Some("text/event-streaming"),
    ] {
        let server = MockServer::start().await;
        let response = if let Some(content_type) = content_type {
            ResponseTemplate::new(200).set_body_raw("SECRET HTML body", content_type)
        } else {
            ResponseTemplate::new(200)
        };
        Mock::given(method("POST"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let client = AvisoClient::builder().base_url(server.uri()).build()?;
        let mut stream = client.watch(WatchRequest::watch("mars"))?;
        let ready = stream.subscribe_ready();
        let error = tokio::time::timeout(Duration::from_secs(2), stream.recv()).await?;
        assert!(matches!(
            &error,
            Some(Err(ClientError::StreamProtocol { .. }))
        ));
        assert!(!format!("{error:?}").contains("SECRET"));
        assert!(!*ready.borrow());
        assert!(stream.recv().await.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn terminal_http_statuses_never_confirm() -> TestResult {
    for status in [401, 403, 404] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let client = AvisoClient::builder().base_url(server.uri()).build()?;
        let mut stream = client.watch(WatchRequest::watch("mars"))?;
        let item = stream.recv().await;
        assert!(
            matches!(item, Some(Err(ClientError::Http { status: actual, .. })) if actual == status)
        );
        assert!(!*stream.subscribe_ready().borrow());
        assert!(stream.recv().await.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn redirect_to_login_is_not_a_subscription() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/watch"))
        .respond_with(ResponseTemplate::new(303).insert_header("location", "/login?token=SECRET"))
        .mount(&server)
        .await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("SECRET", "text/html"))
        .mount(&server)
        .await;
    let client = AvisoClient::builder().base_url(server.uri()).build()?;
    let mut stream = client.watch(WatchRequest::watch("mars"))?;
    let error = stream.recv().await;
    assert!(matches!(
        &error,
        Some(Err(ClientError::StreamProtocol { .. }))
    ));
    assert!(!format!("{error:?}").contains("SECRET"));
    assert!(!*stream.subscribe_ready().borrow());
    Ok(())
}

#[tokio::test]
async fn handshake_must_match_resolved_watch_mode() -> TestResult {
    for (request, opening) in [
        (WatchRequest::watch("mars"), REPLAY),
        (
            WatchRequest::watch_from("mars", ResumeStart::AfterSequence(0)),
            LIVE,
        ),
        (
            WatchRequest::replay_only("mars", ResumeStart::AfterSequence(0)),
            LIVE,
        ),
        (
            WatchRequest::watch("mars"),
            "event: live-notification\ndata: {\"id\":\"mars@1\",\"data\":{\"identifier\":{}}}\n\n",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(opening, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;
        let client = AvisoClient::builder().base_url(server.uri()).build()?;
        let mut stream = client.watch(request)?;
        assert!(matches!(
            stream.recv().await,
            Some(Err(ClientError::StreamProtocol { .. }))
        ));
        assert!(!*stream.subscribe_ready().borrow());
    }
    Ok(())
}

#[tokio::test]
async fn valid_openings_confirm_without_notifications_and_preserve_decode_errors() -> TestResult {
    for (request, opening) in [
        (WatchRequest::watch("mars"), LIVE),
        (
            WatchRequest::watch_from("mars", ResumeStart::AfterSequence(0)),
            REPLAY,
        ),
        (
            WatchRequest::replay_only("mars", ResumeStart::AfterSequence(0)),
            REPLAY,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!("{opening}event: live-notification\ndata: not-json\n\n"),
                "Text/Event-Stream; Charset=UTF-8",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let client = AvisoClient::builder().base_url(server.uri()).build()?;
        let mut stream = client.watch(request)?;
        let mut ready = stream.subscribe_ready();
        assert!(*ready.wait_for(|value| *value).await?);
        assert!(matches!(
            stream.recv().await,
            Some(Err(ClientError::Decode(_)))
        ));
    }
    Ok(())
}

/// A single connection held open until the client closes it. The signal lets
/// tests advance virtual time only after real socket I/O has completed.
async fn held_connection(
    response: String,
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
    let (sent, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await? > 0);
        socket.write_all(response.as_bytes()).await?;
        sent.send(()).ok(); // The test may already have cancelled.
        while socket.read(&mut request).await? != 0 {}
        Ok(())
    });
    Ok((url, received, task))
}

fn streaming_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n",
        body.len()
    )
}

#[tokio::test]
async fn unconfirmed_openings_have_a_fixed_deadline() -> TestResult {
    for response in [
        String::new(),
        streaming_response("event: heartbeat\ndata: {}\n\n"),
        streaming_response("event: unrelated\ndata: {}\n\n"),
    ] {
        let (url, sent, task) = held_connection(response).await?;
        let client = AvisoClient::builder().base_url(url).build()?;
        let mut stream = client.watch(WatchRequest::watch("mars"))?;
        sent.await?;
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(11)).await;
        let result = stream.recv().await;
        assert!(matches!(
            result,
            Some(Err(ClientError::StreamProtocol { .. }))
        ));
        assert!(!*stream.subscribe_ready().borrow());
        tokio::time::resume();
        stream.close().await;
        task.await??;
    }
    Ok(())
}

#[tokio::test]
async fn confirmed_idle_stream_outlives_startup_budget() -> TestResult {
    let (url, sent, task) = held_connection(streaming_response(LIVE)).await?;
    let client = AvisoClient::builder().base_url(url).build()?;
    let mut stream = client
        .watch(WatchRequest::watch("mars").with_startup_timeout(Some(Duration::from_secs(30))))?;
    sent.await?;
    let mut ready = stream.subscribe_ready();
    assert!(*ready.wait_for(|value| *value).await?);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(40)).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), stream.recv())
            .await
            .is_err()
    );
    tokio::time::resume();
    stream.close().await;
    task.await??;
    Ok(())
}

#[tokio::test]
async fn retryable_statuses_share_one_startup_budget() -> TestResult {
    for status in [429, 500, 503] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).insert_header("retry-after", "60"))
            .mount(&server)
            .await;
        let client = AvisoClient::builder().base_url(server.uri()).build()?;
        let mut stream = client.watch(
            WatchRequest::watch("mars").with_startup_timeout(Some(Duration::from_millis(100))),
        )?;
        let result = tokio::time::timeout(Duration::from_secs(2), stream.recv()).await?;
        assert!(
            matches!(result, Some(Err(ClientError::StreamProtocol { message, .. })) if message.contains("startup timeout"))
        );
        assert!(!*stream.subscribe_ready().borrow());
        assert!(stream.recv().await.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn startup_budget_is_not_reapplied_after_reconnect() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(LIVE, "text/event-stream"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
        .mount(&server)
        .await;
    let client = AvisoClient::builder().base_url(server.uri()).build()?;
    let mut stream = client.watch(
        WatchRequest::watch("mars").with_startup_timeout(Some(Duration::from_millis(100))),
    )?;
    let mut ready = stream.subscribe_ready();
    assert!(*ready.wait_for(|value| *value).await?);
    assert!(
        tokio::time::timeout(Duration::from_millis(400), stream.recv())
            .await
            .is_err()
    );
    assert!(*ready.borrow());
    stream.close().await;
    assert!(
        server
            .received_requests()
            .await
            .ok_or("missing requests")?
            .len()
            >= 2
    );
    Ok(())
}

#[tokio::test]
async fn server_error_before_opening_retains_protocol_details() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "event: error\ndata: {\"message\":\"stream rejected\",\"request_id\":\"request-1\"}\n\n",
            "text/event-stream",
        )).mount(&server).await;
    let client = AvisoClient::builder().base_url(server.uri()).build()?;
    let mut stream = client.watch(WatchRequest::watch("mars"))?;
    assert!(matches!(stream.recv().await,
        Some(Err(ClientError::StreamProtocol { message, request_id }))
            if message == "stream rejected" && request_id.as_deref() == Some("request-1")));
    assert!(!*stream.subscribe_ready().borrow());
    Ok(())
}
