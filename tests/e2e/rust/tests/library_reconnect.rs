//! E2E: long-running watch survives the routine `max_duration_reached` reconnect.

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
const CONNECTION_MAX_DURATION_SEC: u64 = 15;

fn polygon_filter() -> BTreeMap<String, Value> {
    BTreeMap::from([("polygon".into(), Value::String(POLYGON.into()))])
}

async fn publish(client: &aviso::AvisoClient, seq: u64) {
    let request = NotificationRequest::new(EVENT_TYPE)
        .with_identifier(BTreeMap::from([
            ("polygon".into(), POLYGON.into()),
            ("date".into(), "20260611".into()),
            ("time".into(), format!("{seq:04}")),
        ]))
        .with_payload(json!({"seq": seq}));
    client.notify(&request).await.unwrap();
}

#[ignore = "requires e2e compose stack; ~20 s wall time for the reconnect cycle"]
#[tokio::test]
async fn rust_listener_survives_max_duration_reached_cut() {
    let client = producer_client().unwrap();

    let mut stream = client
        .watch(WatchRequest::watch(EVENT_TYPE).with_filter(polygon_filter()))
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    let publisher = client.clone();
    let publish_task = tokio::spawn(async move {
        publish(&publisher, 1).await;
        publish(&publisher, 2).await;
        tokio::time::sleep(Duration::from_secs(CONNECTION_MAX_DURATION_SEC + 2)).await;
        publish(&publisher, 3).await;
        publish(&publisher, 4).await;
    });

    let mut received: Vec<u64> = Vec::new();
    let expected: std::collections::HashSet<u64> = [1, 2, 3, 4].into_iter().collect();
    let deadline = Duration::from_secs(CONNECTION_MAX_DURATION_SEC + 10);
    while received
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        != expected
    {
        let item = timeout(deadline, stream.next())
            .await
            .expect("a notification must arrive within the budget")
            .expect("stream must not close before all four items")
            .expect("no terminal error");
        received.push(item.payload.get("seq").and_then(Value::as_u64).unwrap());
    }

    publish_task.await.unwrap();
    let received_set: std::collections::HashSet<u64> = received.iter().copied().collect();
    assert_eq!(
        received_set, expected,
        "all four items must arrive at least once"
    );
    assert!(
        received.len() <= expected.len() + 1,
        "at-most-one duplicate per cut per D2; got {received:?}"
    );
}
