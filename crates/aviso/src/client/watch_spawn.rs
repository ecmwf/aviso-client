use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::AvisoClient;
use crate::ClientError;
use crate::state::ResumeKey;
use crate::watch::{
    CHANNEL_CAPACITY, NotificationStream, WatchRequest, WireWatchRequest, run_supervisor,
};

impl AvisoClient {
    /// Open a watch on the configured server and return a
    /// [`NotificationStream`].
    ///
    /// The call spawns a supervisor task on the ambient Tokio runtime that
    /// owns its own HTTP connection and forwards [`crate::Notification`]s
    /// to the returned stream. The stream is single-consumer; dropping it
    /// cancels the supervisor cooperatively.
    ///
    /// # Errors
    ///
    /// - [`ClientError::Config`] when no Tokio runtime is entered (this
    ///   method requires `tokio::runtime::Handle::try_current()` to
    ///   succeed; the spawn would otherwise panic, which the library does
    ///   not do).
    /// - [`ClientError::Config`] when the request would advance past
    ///   `u64::MAX` on the wire (`AfterSequence(u64::MAX)`).
    ///
    /// Errors observed by the supervisor while the stream is open surface
    /// on the stream itself as `Err(_)` items, not from this method.
    pub fn watch(&self, request: WatchRequest) -> crate::Result<NotificationStream> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ClientError::Config("AvisoClient::watch requires a Tokio runtime".into())
        })?;
        let _ = WireWatchRequest::from_public(&request)?;
        let resume_key = compute_resume_key(&self.base_url, &request)?;
        // Channel capacity controls whether the user-facing
        // "pulling item N+1 implies item N is durable" contract holds.
        // The supervisor commits item N to the store BEFORE attempting
        // to send item N+1; with a capacity > 1, item N could land on
        // disk while item N is still buffered ahead of the consumer,
        // breaking the at-least-once contract on a crash between
        // commit and consumer-pull.
        //
        // When a state store is configured we therefore force capacity
        // to 1: the supervisor's send of N+1 then blocks (TCP
        // backpressure propagates upstream) until the consumer has
        // pulled item N, so the durability claim is preserved.
        //
        // Without a state store there is no durability claim to
        // preserve, and the larger buffer absorbs short consumer
        // pauses without throttling the wire.
        let capacity = if self.state_store.is_some() {
            1
        } else {
            CHANNEL_CAPACITY
        };
        let (tx, rx) = mpsc::channel(capacity);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let parent_cancel = self.parent_drop.subscribe();
        let http = self.http.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth.clone();
        let heartbeat_interval = self.heartbeat_interval;
        let state_store = self.state_store.clone();
        let active_resume_keys = self.active_resume_keys.clone();
        let flush_cursor_on_exit = self.flush_cursor_on_exit;
        // Increment the active-resume-key refcount before spawning, and
        // emit a WARN log if the key was already in use. Refcount
        // semantics preserve the invariant that collisions are reported
        // for every overlapping watch, not just the first pair: a
        // single watch warns 0 times; a second concurrent same-key
        // watch warns once; if one of those exits while another remains
        // active, a third same-key watch must still warn.
        increment_active_key(&active_resume_keys, &resume_key, request.event_type());
        handle.spawn(run_supervisor(
            request,
            http,
            base_url,
            auth,
            heartbeat_interval,
            state_store,
            resume_key,
            tx,
            cancel_rx,
            parent_cancel,
            active_resume_keys,
            flush_cursor_on_exit,
            done_tx,
        ));
        Ok(NotificationStream::new(rx, cancel_tx, done_rx))
    }

    /// Open a watch and drain it through a per-notification handler.
    ///
    /// Convenience wrapper over [`Self::watch`]: opens the stream, calls
    /// `handler(notification).await` for each `Ok(_)` item, and propagates
    /// the first `Err(_)` from either the stream or the handler. Returns
    /// `Ok(())` when the stream ends without errors (the server closed
    /// cleanly or the consumer is replay-only and ran to completion).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use aviso::watch::WatchRequest;
    /// use aviso::{AvisoClient, Result};
    ///
    /// # async fn run(client: AvisoClient) -> Result<()> {
    /// client
    ///     .watch_with_handler(WatchRequest::watch("mars"), |notification| async move {
    ///         println!("got sequence {}", notification.sequence);
    ///         Ok(())
    ///     })
    ///     .await
    /// # }
    /// ```
    ///
    /// Daemon-style consumers prefer this over the raw [`Self::watch`]
    /// stream when they do not need select-loop integration: the
    /// supervisor's reconnect, checkpoint, and trigger semantics are
    /// identical because both surfaces drain the same internal channel.
    ///
    /// If `handler` returns `Err(_)`, the underlying stream is dropped
    /// (which cancels the supervisor cooperatively) and the error
    /// propagates as the method's return value.
    ///
    /// # Errors
    ///
    /// - Any error from [`Self::watch`] is propagated.
    /// - Any `Err(_)` item from the stream is propagated.
    /// - Any `Err(_)` from the handler is propagated, with the supervisor
    ///   cancelled before return.
    pub async fn watch_with_handler<F, Fut>(
        &self,
        request: WatchRequest,
        mut handler: F,
    ) -> crate::Result<()>
    where
        F: FnMut(crate::Notification) -> Fut + Send,
        Fut: std::future::Future<Output = crate::Result<()>> + Send,
    {
        let mut stream = self.watch(request)?;
        loop {
            match stream.recv().await {
                Some(Ok(notification)) => handler(notification).await?,
                Some(Err(e)) => return Err(e),
                None => return Ok(()),
            }
        }
    }
}

/// Increment the active-resume-key refcount for `key`, emit a WARN if
/// the prior count was greater than zero. Mutex poisoning is handled by
/// recovering the inner guard and logging a poison-specific WARN; the
/// refcount itself is best-effort observability, so missing one warn
/// during a poison event is acceptable (and the supervisor still
/// decrements on exit).
pub(crate) fn increment_active_key(
    active: &Arc<Mutex<HashMap<ResumeKey, usize>>>,
    key: &ResumeKey,
    event_type: &str,
) {
    let (mut guard, poisoned) = match active.lock() {
        Ok(g) => (g, false),
        Err(poison) => {
            tracing::warn!(
                event.name = "client.resume.collision.poisoned",
                "resume-key collision tracker mutex is poisoned; refcount continues but \
                 collision WARN is suppressed for this call"
            );
            (poison.into_inner(), true)
        }
    };
    let prior = *guard.get(key).unwrap_or(&0);
    guard.insert(key.clone(), prior + 1);
    drop(guard);
    if prior > 0 && !poisoned {
        tracing::warn!(
            event.name = "client.resume.collision",
            resume_key = %key.as_hex(),
            event_type = event_type,
            "multiple concurrent watch() calls share the same resume key; checkpoint \
             advancement is racy and the affected watches will interleave commits"
        );
    }
}

/// Decrement the active-resume-key refcount for `key`. Called by every
/// supervisor exit path so the tracker stays accurate. Poison handling
/// recovers the inner guard; the entry is removed when the count
/// returns to zero.
pub(crate) fn decrement_active_key(
    active: &Arc<Mutex<HashMap<ResumeKey, usize>>>,
    key: &ResumeKey,
) {
    let mut guard = match active.lock() {
        Ok(g) => g,
        Err(poison) => poison.into_inner(),
    };
    if let Some(count) = guard.get_mut(key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            guard.remove(key);
        }
    }
}

/// Compute the resume key for a watch request against this client's
/// base URL. Surface filter-canonicalisation failures as
/// `ClientError::Config` since they indicate a malformed `WatchRequest`,
/// not a runtime persistence failure.
pub(crate) fn compute_resume_key(
    base_url: &Url,
    request: &WatchRequest,
) -> crate::Result<ResumeKey> {
    let filter_value = serde_json::Value::Object(
        request
            .filter()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    ResumeKey::new(base_url, request.event_type(), &filter_value, None)
        .map_err(|e| ClientError::Config(format!("compute resume key for watch request: {e}")))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on constructor success and mutex guards is the expected diagnostic"
)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn resume_key_collision_refcount_semantics() {
        let active = Arc::new(Mutex::new(HashMap::new()));
        let key = crate::state::ResumeKey::new(
            &url::Url::parse("http://example.com/").unwrap(),
            "mars",
            &serde_json::Value::Object(serde_json::Map::default()),
            None,
        )
        .unwrap();

        let count_at = |a: &Arc<Mutex<HashMap<_, _>>>, k: &crate::state::ResumeKey| -> usize {
            *a.lock().unwrap().get(k).unwrap_or(&0)
        };

        super::increment_active_key(&active, &key, "mars");
        assert_eq!(count_at(&active, &key), 1);

        super::increment_active_key(&active, &key, "mars");
        assert_eq!(count_at(&active, &key), 2);

        super::decrement_active_key(&active, &key);
        assert_eq!(count_at(&active, &key), 1);

        super::increment_active_key(&active, &key, "mars");
        assert_eq!(count_at(&active, &key), 2);

        super::decrement_active_key(&active, &key);
        super::decrement_active_key(&active, &key);
        assert_eq!(count_at(&active, &key), 0);
        assert!(
            !active.lock().unwrap().contains_key(&key),
            "entry must be removed at zero refcount"
        );

        super::increment_active_key(&active, &key, "mars");
        assert_eq!(count_at(&active, &key), 1);
        super::decrement_active_key(&active, &key);
    }
}
