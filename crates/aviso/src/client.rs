//! Public client type and builder.
//!
//! [`AvisoClient`] is the user-facing handle: it owns a `reqwest` HTTP client, the base URL of
//! the `aviso-server`, and an optional [`AuthProvider`]. Construct it with
//! [`AvisoClient::builder`].
//!
//! The base URL is normalized at build time to end with a `/`. That makes
//! [`url::Url::join`] consistently treat endpoint paths (`api/v1/notification`, ...) as relative
//! and preserves any path prefix the operator picks (for example a reverse proxy that mounts
//! `aviso-server` under `/aviso`).

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client as HttpClient;
use tokio::sync::{mpsc, oneshot, watch};
use url::Url;

use crate::ClientError;
use crate::auth::AuthProvider;
use crate::watch::{
    CHANNEL_CAPACITY, NotificationStream, WatchRequest, WireWatchRequest, run_supervisor,
};

/// Cascading-cancellation handle shared by all clones of an [`AvisoClient`].
///
/// Each `AvisoClient` carries `Arc<DropGuard>`; every supervisor spawned by a
/// `watch()` call holds a [`watch::Receiver<bool>`] derived from the same
/// sender. When the last `Arc<DropGuard>` is dropped (the last `AvisoClient`
/// clone goes away), [`DropGuard::drop`] fires `sender.send(true)`. Every
/// supervisor's `watch::Receiver::changed()` await observes the value flip
/// within one event-loop tick and the supervisor exits its loop.
///
/// `pub(crate)` because it is an implementation detail of the cancel cascade.
/// The publicly visible behaviour is documented on [`AvisoClient`] itself.
pub(crate) struct DropGuard {
    sender: watch::Sender<bool>,
}

impl DropGuard {
    /// Construct a fresh guard plus one receiver. Subsequent supervisors clone
    /// the receiver via [`Self::subscribe`].
    pub(crate) fn new() -> (Arc<Self>, watch::Receiver<bool>) {
        let (sender, receiver) = watch::channel(false);
        (Arc::new(Self { sender }), receiver)
    }

    /// Clone a new `watch::Receiver` from the shared sender.
    #[allow(
        dead_code,
        reason = "consumed by the watch supervisor's parent-cancel select arms in a follow-up commit; lands before any production consumer so the cancel-cascade type and its tests can be reviewed in isolation"
    )]
    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        // Best-effort: if every receiver has been dropped already (which the
        // type system permits but normal flow should not produce, because the
        // sender owner is `Arc::new(self)` and outlives any single receiver),
        // the send call returns `Err` which we silently drop. The guarantee
        // we care about is "the value flips to true while at least one
        // receiver is still alive", and that holds.
        let _ = self.sender.send(true);
    }
}

/// Top-level handle to an `aviso-server`.
///
/// Cheap to clone; cloned handles share the same underlying HTTP connection pool and auth
/// provider behind reference counts. Marked `#[non_exhaustive]` so future fields (for example
/// per-request middleware) can be added without breaking downstream pattern matches.
#[derive(Clone)]
#[non_exhaustive]
pub struct AvisoClient {
    http: HttpClient,
    base_url: Url,
    auth: Option<Arc<dyn AuthProvider>>,
    /// Cascading cancellation token shared by all clones. When the last
    /// clone drops, all child supervisors observe the value flip and exit.
    /// See [`DropGuard`] for the mechanism.
    #[allow(
        dead_code,
        reason = "field is read by the watch supervisor's spawn path in a follow-up commit; the `Clone` derive consumes it on every clone, but rustc does not count derive expansions as reads for dead-code analysis"
    )]
    parent_drop: Arc<DropGuard>,
    /// Expected SSE heartbeat cadence; see
    /// [`AvisoClientBuilder::heartbeat_interval`] for the default and the
    /// budget formula.
    heartbeat_interval: Duration,
}

impl std::fmt::Debug for AvisoClient {
    /// Custom `Debug` because the derived form prints `reqwest::Client` internals (proxy
    /// configuration, default headers, ...) and `Url` userinfo verbatim. Both can leak secrets
    /// in log output. The custom impl elides the HTTP client entirely and prints a sanitized
    /// base URL with any userinfo stripped.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sanitized = self.base_url.clone();
        let _ = sanitized.set_username("");
        let _ = sanitized.set_password(None);
        f.debug_struct("AvisoClient")
            .field("base_url", &sanitized.as_str())
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl AvisoClient {
    /// Starts a new [`AvisoClientBuilder`]. The `base_url` must be set before
    /// [`AvisoClientBuilder::build`].
    pub fn builder() -> AvisoClientBuilder {
        AvisoClientBuilder::default()
    }

    /// Returns the normalized base URL.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns the auth provider, if one was configured.
    #[must_use]
    pub fn auth(&self) -> Option<&Arc<dyn AuthProvider>> {
        self.auth.as_ref()
    }

    /// Internal accessor for the HTTP client; used by sibling modules that build requests.
    pub(crate) fn http(&self) -> &HttpClient {
        &self.http
    }

    /// Joins a relative endpoint path onto the base URL.
    ///
    /// The path must not start with `/`. An absolute path would otherwise wipe out any
    /// reverse-proxy prefix in the base URL: `Url::join("https://gw/aviso/", "/api/v1/x")` is
    /// `https://gw/api/v1/x`, not `https://gw/aviso/api/v1/x`.
    pub(crate) fn endpoint(&self, relative_path: &str) -> crate::Result<Url> {
        self.base_url.join(relative_path).map_err(|e| {
            ClientError::Config(format!(
                "build endpoint url from base {} and path {relative_path:?}: {e}",
                self.base_url
            ))
        })
    }

    /// Attaches the configured `Authorization` header (if any) to a request builder.
    pub(crate) async fn attach_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> crate::Result<reqwest::RequestBuilder> {
        if let Some(auth) = self.auth() {
            let value = auth.authorization_header().await?;
            Ok(builder.header(reqwest::header::AUTHORIZATION, value))
        } else {
            Ok(builder)
        }
    }

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
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let http = self.http.clone();
        let base_url = self.base_url.clone();
        let auth = self.auth.clone();
        let heartbeat_interval = self.heartbeat_interval;
        handle.spawn(run_supervisor(
            request,
            http,
            base_url,
            auth,
            heartbeat_interval,
            tx,
            cancel_rx,
        ));
        Ok(NotificationStream::new(rx, cancel_tx))
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

    /// Sends a request with the shared `401 -> refresh -> retry once` contract per D8.
    ///
    /// The caller supplies a closure that builds a fresh `RequestBuilder` each time it is called.
    /// The closure runs at most twice: once for the initial attempt, and again only if the first
    /// response was `401` and an auth provider is configured to refresh. A second `401` is
    /// returned to the caller without further retries.
    pub(crate) async fn send_with_refresh<F>(
        &self,
        mut build: F,
    ) -> crate::Result<reqwest::Response>
    where
        F: FnMut(&HttpClient) -> reqwest::RequestBuilder,
    {
        let first = self.attach_auth(build(self.http())).await?;
        let response = first.send().await.map_err(ClientError::from)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(auth) = self.auth() {
                drop(response);
                auth.refresh().await?;
                let retry = self.attach_auth(build(self.http())).await?;
                return retry.send().await.map_err(ClientError::from);
            }
        }
        Ok(response)
    }
}

/// Parses a JSON response, returning `T` on success and [`ClientError::Http`] (with verbatim body
/// and `X-Request-ID`) on any non-success status. Shared by notify/schema.
pub(crate) async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> crate::Result<T> {
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from);
    let body = response.bytes().await?;
    if status.is_success() {
        let parsed: T = serde_json::from_slice(&body)?;
        Ok(parsed)
    } else {
        let body_str = String::from_utf8_lossy(&body).into_owned();
        Err(ClientError::Http {
            status: status.as_u16(),
            body: body_str,
            request_id,
        })
    }
}

/// Validates a user-supplied path segment before it is spliced into a relative URL via
/// [`format!`]. Rejects:
///
/// - empty input,
/// - the path-traversal tokens `.` and `..` (which would normalize into a sibling endpoint after
///   [`url::Url::join`] resolves the relative reference),
/// - URL structural characters (`/`, `\`, `?`, `#`) that would change the request target,
/// - any ASCII control character.
///
/// `@` and `%` are deliberately permitted because the server's notification ids use `@` literally
/// and we want callers to pass already-encoded segments through verbatim.
pub(crate) fn validate_path_segment(segment: &str) -> crate::Result<()> {
    if segment.is_empty() {
        return Err(ClientError::Config(
            "dynamic path segment must not be empty".into(),
        ));
    }
    if segment == "." || segment == ".." {
        return Err(ClientError::Config(format!(
            "dynamic path segment {segment:?} would traverse into a sibling endpoint"
        )));
    }
    for c in segment.chars() {
        if matches!(c, '/' | '\\' | '?' | '#') || c.is_control() {
            return Err(ClientError::Config(format!(
                "dynamic path segment {segment:?} contains forbidden character {c:?}"
            )));
        }
    }
    Ok(())
}

/// Like [`parse_json_response`] but for endpoints that return either `204 No Content` or a body
/// the client does not need. Success drains the body so the connection can be pooled and any
/// late transport failure during that drain (truncated read, broken connection mid-body) is
/// surfaced rather than silently discarded.
pub(crate) async fn parse_json_response_optional(response: reqwest::Response) -> crate::Result<()> {
    let status = response.status();
    if status.is_success() {
        let _body = response.bytes().await?;
        Ok(())
    } else {
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .map(String::from);
        let body = response.bytes().await?;
        let body_str = String::from_utf8_lossy(&body).into_owned();
        Err(ClientError::Http {
            status: status.as_u16(),
            body: body_str,
            request_id,
        })
    }
}

/// Builder for [`AvisoClient`].
#[derive(Debug, Default)]
#[must_use]
pub struct AvisoClientBuilder {
    base_url: Option<String>,
    auth: Option<Arc<dyn AuthProvider>>,
    timeout: Option<Duration>,
    user_agent: Option<String>,
    heartbeat_interval: Option<Duration>,
}

impl AvisoClientBuilder {
    /// Sets the `aviso-server` base URL. Required.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Sets the auth provider. Optional; the client sends no `Authorization` header when unset,
    /// which is the right configuration for anonymous-access streams on `aviso-server`.
    pub fn auth(mut self, auth: Arc<dyn AuthProvider>) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Sets the per-request HTTP timeout. Optional; defaults to whatever `reqwest::Client`
    /// itself defaults to (no timeout in current versions).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the `User-Agent` header. Optional; defaults to `"aviso/<crate-version>"`.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Sets the expected SSE heartbeat cadence on the watch endpoint.
    ///
    /// The watch supervisor uses this to compute its heartbeat-starvation
    /// budget per D2: a stream is declared silent (and reconnected with
    /// exponential backoff) if no SSE event of any kind arrives within
    /// `max(3 * interval, interval + 30s)`. Defaults to 30 seconds, which
    /// matches the default `aviso-server` configuration.
    ///
    /// Set this to match a non-default server-side heartbeat configuration.
    /// Setting it too low will false-positive on healthy quiet streams;
    /// setting it too high delays detection of silently-dead connections
    /// (NAT timeout, half-open socket after sleep, intermediate-proxy
    /// restart).
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = Some(interval);
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Config`] when `base_url` is missing or not a valid URL, or
    /// when the underlying `reqwest::Client` cannot be built.
    pub fn build(self) -> crate::Result<AvisoClient> {
        let raw = self
            .base_url
            .ok_or_else(|| ClientError::Config("AvisoClient requires a base_url".into()))?;
        let mut base_url = Url::parse(&raw)
            .map_err(|e| ClientError::Config(format!("invalid base_url {raw:?}: {e}")))?;
        if !base_url.path().ends_with('/') {
            let normalized = format!("{}/", base_url.path());
            base_url.set_path(&normalized);
        }
        let user_agent = self
            .user_agent
            .unwrap_or_else(|| format!("aviso/{}", crate::VERSION));
        let mut http_builder = HttpClient::builder().user_agent(user_agent);
        if let Some(timeout) = self.timeout {
            http_builder = http_builder.timeout(timeout);
        }
        let http = http_builder
            .build()
            .map_err(|e| ClientError::Config(format!("failed to build HTTP client: {e}")))?;
        let (parent_drop, _initial_receiver) = DropGuard::new();
        let heartbeat_interval = self
            .heartbeat_interval
            .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL);
        Ok(AvisoClient {
            http,
            base_url,
            auth: self.auth,
            parent_drop,
            heartbeat_interval,
        })
    }
}

/// Default expected SSE heartbeat cadence, matching the default
/// `aviso-server` configuration. The watchdog budget at this default is
/// `max(3 * 30s, 30s + 30s) = 90s`.
const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on constructor success and on assertion-shaped awaits are the expected diagnostics"
)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::AvisoClient;
    use crate::auth::Bearer;

    #[test]
    fn builder_requires_base_url() {
        let err = AvisoClient::builder().build().unwrap_err();
        assert!(matches!(err, crate::ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn builder_rejects_invalid_url() {
        let err = AvisoClient::builder()
            .base_url("not a url")
            .build()
            .unwrap_err();
        assert!(matches!(err, crate::ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn builder_normalizes_base_url_to_trailing_slash() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .build()
            .unwrap();
        assert!(client.base_url().path().ends_with('/'));
    }

    #[test]
    fn builder_preserves_existing_trailing_slash() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000/")
            .build()
            .unwrap();
        assert_eq!(client.base_url().as_str(), "http://localhost:8000/");
    }

    #[test]
    fn builder_preserves_path_prefix_with_trailing_slash() {
        let client = AvisoClient::builder()
            .base_url("https://gw.example.org/aviso")
            .build()
            .unwrap();
        assert_eq!(client.base_url().as_str(), "https://gw.example.org/aviso/");
    }

    #[test]
    fn auth_defaults_to_none() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .build()
            .unwrap();
        assert!(client.auth().is_none());
    }

    #[test]
    fn auth_is_set_when_provided() {
        let provider =
            Arc::new(Bearer::new("token").unwrap()) as Arc<dyn crate::auth::AuthProvider>;
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .auth(provider)
            .build()
            .unwrap();
        assert!(client.auth().is_some());
    }

    #[test]
    fn endpoint_joins_relative_paths() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .build()
            .unwrap();
        let url = client.endpoint("api/v1/notification").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8000/api/v1/notification");
    }

    #[test]
    fn endpoint_join_respects_proxy_path_prefix() {
        let client = AvisoClient::builder()
            .base_url("https://gw.example.org/aviso")
            .build()
            .unwrap();
        let url = client.endpoint("api/v1/notification").unwrap();
        assert_eq!(
            url.as_str(),
            "https://gw.example.org/aviso/api/v1/notification"
        );
    }

    #[test]
    fn client_is_cheap_to_clone() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .build()
            .unwrap();
        let _copy = client.clone();
    }

    #[test]
    fn debug_does_not_leak_auth_token() {
        let provider = Arc::new(Bearer::new("super-secret-jwt-do-not-leak").unwrap())
            as Arc<dyn crate::auth::AuthProvider>;
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .auth(provider)
            .build()
            .unwrap();
        let formatted = format!("{client:?}");
        assert!(
            !formatted.contains("super-secret-jwt-do-not-leak"),
            "AvisoClient Debug must not leak the auth token: {formatted}"
        );
    }

    #[test]
    fn debug_strips_userinfo_from_base_url() {
        let client = AvisoClient::builder()
            .base_url("https://operator:hunter2@aviso.example.org")
            .build()
            .unwrap();
        let formatted = format!("{client:?}");
        assert!(
            !formatted.contains("hunter2"),
            "AvisoClient Debug must strip password from base_url: {formatted}"
        );
        assert!(
            !formatted.contains("operator"),
            "AvisoClient Debug must strip username from base_url: {formatted}"
        );
    }

    #[tokio::test]
    async fn drop_guard_fires_when_last_clone_drops() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .build()
            .unwrap();
        let mut receiver = client.parent_drop.subscribe();
        assert!(!*receiver.borrow_and_update());
        let clone = client.clone();
        drop(client);
        assert!(
            !*receiver.borrow_and_update(),
            "guard must NOT fire while another clone is alive"
        );
        drop(clone);
        let observed = tokio::time::timeout(Duration::from_millis(100), receiver.changed())
            .await
            .expect("guard must fire within 100ms of last clone drop");
        assert!(observed.is_ok());
        assert!(*receiver.borrow_and_update());
    }

    #[tokio::test]
    async fn drop_guard_broadcasts_to_multiple_subscribers() {
        let client = AvisoClient::builder()
            .base_url("http://localhost:8000")
            .build()
            .unwrap();
        let mut a = client.parent_drop.subscribe();
        let mut b = client.parent_drop.subscribe();
        let mut c = client.parent_drop.subscribe();
        drop(client);
        for rx in [&mut a, &mut b, &mut c] {
            let observed = tokio::time::timeout(Duration::from_millis(100), rx.changed()).await;
            assert!(
                observed.is_ok_and(|r| r.is_ok()),
                "every subscriber must observe the drop"
            );
            assert!(*rx.borrow_and_update());
        }
    }
}
