// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::*;

#[tokio::test]
async fn eof_resets_backoff_only_after_a_validated_handshake() {
    for (body, expected_retries, expected_ready) in [
        ("", 4, false),
        (
            "event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n",
            0,
            true,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let base_url = url::Url::parse(&server.uri()).unwrap();
        let resume_key = ResumeKey::new(&base_url, "mars", &json!({}), None).unwrap();
        let request = WatchRequest::watch("mars");
        let mut state = crate::watch::WatchState::watch(None);
        let mut policy = None;
        let mut cursor = None;
        let mut pending = None;
        let mut retries = 4;
        let (tx, _rx) = mpsc::channel(1);
        let (_cancel_tx, mut cancel) = oneshot::channel();
        let (_parent_tx, mut parent) = tokio::sync::watch::channel(false);
        let (ready, ready_rx) = tokio::sync::watch::channel(false);
        let outcome = super::super::connection::run_one_connection(
            &mut state,
            &mut policy,
            &request,
            None,
            &mut cursor,
            &mut pending,
            None,
            &resume_key,
            &reqwest::Client::new(),
            &base_url,
            None,
            Duration::from_secs(30),
            &mut retries,
            &mut [],
            &tx,
            &mut cancel,
            &mut parent,
            &ready,
        )
        .await;
        assert!(matches!(
            outcome,
            super::super::ConnectionOutcome::UnexpectedEof
        ));
        assert_eq!(retries, expected_retries);
        assert_eq!(*ready_rx.borrow(), expected_ready);
    }
}

#[tokio::test]
async fn connection_closing_terminates_before_any_following_frames() {
    // Even if the server keeps writing notifications after a
    // `connection-closing` frame on the same wire (which it should not,
    // but which is exactly the bug class the supervisor must guard
    // against), the supervisor must stop at the close frame and not
    // surface the trailing notifications.
    let server = MockServer::start().await;
    let body = format!(
        "{}{}{}",
        sse_chunk("live-notification", cloud_event("mars", 1)),
        sse_chunk("connection-closing", closing("end_of_stream")),
        sse_chunk("live-notification", cloud_event("mars", 99)),
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |request: &wiremock::Request| {
            let value: serde_json::Value = request.body_json().unwrap();
            opened_stream(&body, value.get("from_id").is_some())
        })
        .mount(&server)
        .await;
    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(first.sequence, 1);
    // Sequence 99 (after the close frame) must not arrive. We give the
    // supervisor a small window to misbehave, then drop the cancel to
    // break the reconnect loop.
    let next = tokio::time::timeout(Duration::from_millis(150), rx.recv()).await;
    match next {
        Err(_) => {
            // Timeout: nothing more arrived. The expected outcome.
        }
        Ok(Some(Ok(n))) => {
            assert_ne!(
                n.sequence, 99,
                "the post-close-frame notification must NOT surface"
            );
        }
        Ok(other) => panic!("unexpected receive: {other:?}"),
    }
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}

#[tokio::test]
async fn unexpected_eof_without_close_frame_does_not_surface_fatal_error() {
    // EOF without close-frame is reclassified as routine transport
    // loss; the supervisor reconnects rather than surfacing a terminal
    // error. The end-to-end behaviour is covered by an integration
    // test (`unexpected_eof_triggers_reconnect`); this unit test pins
    // the negative invariant: no `Err(_)` item ever surfaces.
    let server = MockServer::start().await;
    let body = sse_chunk("live-notification", cloud_event("mars", 1));
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |request: &wiremock::Request| {
            let value: serde_json::Value = request.body_json().unwrap();
            opened_stream(&body, value.get("from_id").is_some())
        })
        .mount(&server)
        .await;
    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(first.sequence, 1);
    // After the first notification the server EOFs; the supervisor
    // reconnects. The second connection serves the same body (mock
    // does not differentiate), so the next item the consumer sees
    // would be another notification (possibly flagged by GapGuard
    // because sequence 1 is observed-before-expected, which the
    // GapGuard tolerates as a backwards-jump). We do not assert on
    // the second item's exact shape here; we only assert that no
    // `Err(StreamProtocol)` surfaces.
    let next = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
    if let Ok(Some(Err(e))) = next {
        panic!("EOF must not surface as a terminal error, got: {e:?}");
    }
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}
