use super::*;

#[tokio::test]
async fn drop_cancel_oneshot_makes_supervisor_exit_within_bounded_time() {
    // The supervisor must observe cancellation even while it is parked
    // on a bounded-channel `tx.send().await` because the consumer fell
    // behind. The test constructs that exact situation: a response body
    // carrying many more notifications than the channel capacity, no
    // terminating frame, a consumer that takes one item and then stops.
    // Once the channel fills, the supervisor blocks on `send_or_cancel`.
    // Dropping the cancel sender then fires the `select!` arm and the
    // supervisor exits cleanly within 500 ms.
    let server = MockServer::start().await;
    let notification_count = super::CHANNEL_CAPACITY + 64;
    let mut body = String::new();
    for n in 1..=notification_count {
        body.push_str(&sse_chunk(
            "live-notification",
            cloud_event("mars", n as u64),
        ));
    }
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

    let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("first notification should arrive promptly")
        .expect("channel must not close before the first item");
    assert!(
        first.is_ok(),
        "first item should be Ok(Notification): {first:?}"
    );

    drop(cancel_tx);

    tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .expect("supervisor must exit within 500ms of cancellation")
        .expect("supervisor task panicked");

    drop(rx);
}

#[tokio::test]
async fn send_or_cancel_returns_err_when_cancel_fires_first() {
    let (tx, _rx) = mpsc::channel(1);
    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    drop(cancel_tx);
    let (_parent_tx, mut parent_rx) = tokio::sync::watch::channel(false);
    let result = send_or_cancel(
        &tx,
        Err(ClientError::Config("dummy".into())),
        &mut cancel_rx,
        &mut parent_rx,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn send_or_cancel_returns_err_when_receiver_is_dropped() {
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    let (_cancel_tx, mut cancel_rx) = oneshot::channel();
    let (_parent_tx, mut parent_rx) = tokio::sync::watch::channel(false);
    let result = send_or_cancel(
        &tx,
        Err(ClientError::Config("dummy".into())),
        &mut cancel_rx,
        &mut parent_rx,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn send_or_cancel_returns_err_when_parent_cancel_fires() {
    let (tx, _rx) = mpsc::channel::<Result<Notification, ClientError>>(1);
    // Fill the channel so the next send parks.
    tx.send(Err(ClientError::Config("pad".into())))
        .await
        .unwrap();
    let (_cancel_tx, mut cancel_rx) = oneshot::channel();
    let (parent_tx, mut parent_rx) = tokio::sync::watch::channel(false);
    // tx has capacity 0 with a receiver alive but never reading; tx.send
    // would park forever. Firing parent_tx must unblock send_or_cancel
    // via its new arm. The test races the send against a parallel drop.
    let driver = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(parent_tx);
    };
    let send = send_or_cancel(
        &tx,
        Err(ClientError::Config("dummy".into())),
        &mut cancel_rx,
        &mut parent_rx,
    );
    let (result, ()) = tokio::time::timeout(Duration::from_millis(200), async {
        tokio::join!(send, driver)
    })
    .await
    .expect("send_or_cancel must observe parent-drop within 200ms");
    assert!(result.is_err());
}
