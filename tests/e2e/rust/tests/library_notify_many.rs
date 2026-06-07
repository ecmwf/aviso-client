//! E2E: `AvisoClient::notify_many` batch publish + watch roundtrip against the local stack.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic on unexpected variant is the standard test diagnostic"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use aviso::NotificationRequest;
use aviso::watch::WatchRequest;
use aviso_e2e::producer_client;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::time::timeout;

const POLYGON: &str = "0,20,1,20,1,21,0,20";
const EVENT_TYPE: &str = "test_polygon";

fn polygon_filter() -> BTreeMap<String, Value> {
    BTreeMap::from([("polygon".into(), Value::String(POLYGON.into()))])
}

fn request(seq: u64) -> NotificationRequest {
    let identifier = BTreeMap::from([
        ("polygon".into(), POLYGON.into()),
        ("date".into(), "20260611".into()),
        ("time".into(), format!("{seq:04}")),
    ]);
    NotificationRequest::new(EVENT_TYPE)
        .with_identifier(identifier)
        .with_payload(json!({ "seq": seq }))
}

#[ignore = "requires e2e compose stack; run via stack.sh up"]
#[tokio::test]
async fn rust_library_notify_many_publishes_and_listens() {
    let client = producer_client().unwrap();

    let mut stream = client
        .watch(WatchRequest::watch(EVENT_TYPE).with_filter(polygon_filter()))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    let requests: Vec<NotificationRequest> = (1..=3_u64).map(request).collect();
    let results = client.notify_many(&requests, 0).await;
    assert_eq!(results.len(), 3);
    for result in &results {
        result.as_ref().expect("each item published successfully");
    }

    let mut received = Vec::with_capacity(3);
    for _ in 0..3 {
        let item = timeout(Duration::from_secs(10), stream.next())
            .await
            .expect("notification within 10s")
            .expect("stream must not close before three items")
            .expect("no terminal error");
        received.push(item.payload.get("seq").and_then(Value::as_u64).unwrap());
    }
    received.sort_unstable();

    assert_eq!(received, vec![1, 2, 3]);
}
