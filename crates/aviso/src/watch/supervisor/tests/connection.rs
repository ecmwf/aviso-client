use super::*;

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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
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
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
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
