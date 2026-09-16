// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::*;

#[tokio::test]
async fn drain_frames_mapping_covers_live_notification_with_cloudevent() {
    let server = MockServer::start().await;
    let mut event = cloud_event("mars", 7);
    event["data"]["identifier"]["point_cloud"] = json!([[46.0, 8.0], [47.0, 9.0]]);
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", event),
        sse_chunk("connection-closing", closing("end_of_stream"))
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;

    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(first.event_type, "mars");
    assert_eq!(first.sequence, 7);
    assert_eq!(
        first.identifier["point_cloud"],
        json!([[46.0, 8.0], [47.0, 9.0]])
    );
    // In watch mode end_of_stream reconnects; drop the cancel to break
    // the reconnect loop and let the supervisor exit.
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}

#[tokio::test]
async fn drain_frames_mapping_treats_connection_established_marker_as_control() {
    let server = MockServer::start().await;
    let marker = json!({
        "type": "connection_established",
        "topic": "mars",
        "timestamp": "2026-05-17T12:00:00Z",
        "connection_will_close_in_seconds": 3600u64,
        "request_id": "req-est"
    });
    let body = format!(
        "{}{}{}",
        sse_chunk("live-notification", marker),
        sse_chunk("live-notification", cloud_event("mars", 1)),
        sse_chunk("connection-closing", closing("end_of_stream"))
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(
        first.sequence, 1,
        "the connection_established marker must not emit a notification"
    );
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}

#[tokio::test]
async fn drain_frames_mapping_does_not_confuse_payload_substring_with_marker() {
    // Regression: the disambiguation must read the *top-level* `type`
    // field; a CloudEvent whose payload contains the literal string
    // `"connection_established"` must still decode as a Notification.
    let server = MockServer::start().await;
    let ev = cloud_event_with_payload(
        "mars",
        5,
        json!({ "free_text": "connection_established was emitted earlier today" }),
    );
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", ev),
        sse_chunk("connection-closing", closing("end_of_stream"))
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(first.event_type, "mars");
    assert_eq!(first.sequence, 5);
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}

#[tokio::test]
async fn drain_frames_mapping_terminates_on_error_event_with_stream_protocol_error() {
    let server = MockServer::start().await;
    let err = json!({
        "error": "stream_processing_failed",
        "message": "boom",
        "topic": "mars",
        "request_id": "req-err"
    });
    let body = sse_chunk("error", err);
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, _cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let item = rx.recv().await.unwrap();
    match item {
        Err(ClientError::StreamProtocol {
            message,
            request_id,
        }) => {
            assert_eq!(message, "boom");
            assert_eq!(request_id.as_deref(), Some("req-err"));
        }
        other => panic!("expected StreamProtocol, got {other:?}"),
    }
    assert!(rx.recv().await.is_none());
    handle.await.unwrap();
}

#[tokio::test]
async fn drain_frames_mapping_terminates_on_unknown_connection_closing_reason() {
    let server = MockServer::start().await;
    let body = sse_chunk(
        "connection-closing",
        closing("future_reason_we_did_not_anticipate"),
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, _cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let item = rx.recv().await.unwrap();
    assert!(
        matches!(item, Err(ClientError::StreamProtocol { .. })),
        "got {item:?}"
    );
    assert!(rx.recv().await.is_none());
    handle.await.unwrap();
}

#[tokio::test]
async fn drain_frames_mapping_handles_heartbeat_as_observation_only() {
    let server = MockServer::start().await;
    let heartbeat = json!({ "timestamp": "2026-05-17T12:00:00Z", "topic": "mars" });
    let body = format!(
        "{}{}{}",
        sse_chunk("heartbeat", heartbeat),
        sse_chunk("live-notification", cloud_event("mars", 1)),
        sse_chunk("connection-closing", closing("end_of_stream"))
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(first.sequence, 1);
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}

#[tokio::test]
async fn drain_frames_mapping_terminates_on_replay_limit_reached_with_history_gap() {
    let server = MockServer::start().await;
    let limit = json!({
        "type": "notification_replay_limit_reached",
        "topic": "mars",
        "max_allowed": 1000u64,
        "timestamp": "2026-05-17T12:00:00Z"
    });
    let body = sse_chunk("replay-control", limit);
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, _cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let item = rx.recv().await.unwrap();
    match item {
        Err(ClientError::HistoryGap {
            reason: GapReason::ReplayLimitReached { max_allowed },
        }) => {
            assert_eq!(max_allowed, 1000);
        }
        other => panic!("expected HistoryGap{{ReplayLimitReached}}, got {other:?}"),
    }
    assert!(rx.recv().await.is_none());
    handle.await.unwrap();
}

#[tokio::test]
async fn replay_limit_reached_without_max_allowed_surfaces_stream_protocol_error() {
    // Defending against a server protocol regression: if the
    // `notification_replay_limit_reached` payload ever drops the
    // `max_allowed` field, the client must surface a typed protocol
    // error instead of silently emitting a misleading
    // `ReplayLimitReached { max_allowed: 0 }`.
    let server = MockServer::start().await;
    let limit = json!({
        "type": "notification_replay_limit_reached",
        "topic": "mars",
        "timestamp": "2026-05-17T12:00:00Z"
    });
    let body = sse_chunk("replay-control", limit);
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, _cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let item = rx.recv().await.unwrap();
    match item {
        Err(ClientError::StreamProtocol { message, .. }) => {
            assert!(
                message.contains("max_allowed"),
                "message should name the missing field: {message}"
            );
        }
        other => panic!("expected StreamProtocol, got {other:?}"),
    }
    assert!(rx.recv().await.is_none());
    handle.await.unwrap();
}

#[tokio::test]
async fn drain_frames_mapping_silently_ignores_unknown_sse_event_types() {
    let server = MockServer::start().await;
    let body = format!(
        "{}{}{}",
        sse_chunk("future-event-we-do-not-know", json!({ "anything": true })),
        sse_chunk("live-notification", cloud_event("mars", 1)),
        sse_chunk("connection-closing", closing("end_of_stream"))
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(opened_stream(&body, false))
        .mount(&server)
        .await;
    let (mut rx, cancel_tx, handle, _parent_drop) =
        start_supervisor(&server, WatchRequest::watch("mars"));
    let first = rx.recv().await.unwrap().unwrap();
    assert_eq!(first.sequence, 1);
    drop(cancel_tx);
    let join_result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let join_result = join_result.expect("supervisor must exit within 2s of cancel-drop");
    join_result.expect("supervisor task must not panic");
}
