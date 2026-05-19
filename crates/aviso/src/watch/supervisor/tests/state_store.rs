use super::*;

#[tokio::test]
async fn prev_notification_is_committed_before_current_triggers_run() {
    use crate::state::MemoryStore;
    use crate::watch::Trigger;
    use tokio::time::timeout;

    let server = MockServer::start().await;
    let body = format!(
        "{}{}",
        sse_chunk("live-notification", cloud_event("mars", 1)),
        sse_chunk("live-notification", cloud_event("mars", 2)),
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

    let store: Arc<dyn StateStore> = Arc::new(MemoryStore::new());
    let (trigger, _counter) = Trigger::test_fail_on_call(2, 0, true);
    let request = WatchRequest::watch("mars").with_triggers(vec![trigger]);
    let (mut rx, _cancel, handle, _drop) =
        start_supervisor_with_store(&server, request, Some(store.clone()));

    let first = timeout(Duration::from_secs(5), rx.recv()).await;
    let first = first
        .expect("first notification arrives within 5s")
        .expect("channel still open")
        .expect("first notification is Ok");
    assert_eq!(first.sequence, 1);

    let second = timeout(Duration::from_secs(5), rx.recv()).await;
    let second = second
        .expect("trigger failure arrives within 5s")
        .expect("channel still open");
    match second {
        Err(ClientError::TriggerFailed { .. }) => {}
        other => panic!("expected TriggerFailed on second item, got {other:?}"),
    }
    let none = timeout(Duration::from_secs(2), rx.recv()).await;
    let none = none.expect("channel closes within 2s after terminal error");
    assert!(none.is_none());

    timeout(Duration::from_secs(2), handle)
        .await
        .expect("supervisor exits within 2s")
        .expect("supervisor task must not panic");

    let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
    let resume_key = ResumeKey::new(&base_url, "mars", &json!({}), None).unwrap();
    let checkpoint = store.get(&resume_key).await.unwrap();
    let cp = checkpoint.expect("checkpoint for N=1 should be persisted");
    assert_eq!(
        cp.last_committed_sequence, 1,
        "N=1 must have been committed before N=2 triggers ran"
    );
}
