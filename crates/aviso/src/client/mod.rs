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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client as HttpClient;
use tokio::sync::watch;
use url::Url;

use crate::ClientError;
use crate::auth::AuthProvider;
use crate::state::{ResumeKey, StateStore};

mod builder;
mod helpers;
mod watch_spawn;

pub use builder::AvisoClientBuilder;
pub(crate) use helpers::{
    parse_json_response, parse_json_response_optional, validate_path_segment,
};
pub(crate) use watch_spawn::{compute_resume_key, decrement_active_key, increment_active_key};

type ActiveResumeKeys = Arc<Mutex<HashMap<ResumeKey, usize>>>;

const _: fn(&ActiveResumeKeys, &ResumeKey, &str) = increment_active_key;
const _: fn(&Url, &crate::watch::WatchRequest) -> crate::Result<ResumeKey> = compute_resume_key;

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
    pub(super) fn new() -> (Arc<Self>, watch::Receiver<bool>) {
        let (sender, receiver) = watch::channel(false);
        (Arc::new(Self { sender }), receiver)
    }

    /// Clone a new `watch::Receiver` from the shared sender.
    pub(super) fn subscribe(&self) -> watch::Receiver<bool> {
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
    pub(super) http: HttpClient,
    pub(super) base_url: Url,
    pub(super) auth: Option<Arc<dyn AuthProvider>>,
    /// Cascading cancellation token shared by all clones. When the last
    /// clone drops, all child supervisors observe the value flip and exit.
    /// See [`DropGuard`] for the mechanism.
    pub(super) parent_drop: Arc<DropGuard>,
    /// Expected SSE heartbeat cadence; see
    /// [`AvisoClientBuilder::heartbeat_interval`] for the default and the
    /// budget formula.
    pub(super) heartbeat_interval: Duration,
    /// Optional state store for persistent resume across process restarts.
    /// See [`AvisoClientBuilder::state_store`].
    pub(super) state_store: Option<Arc<dyn StateStore>>,
    /// Per-client refcount of currently-active watch supervisors keyed by
    /// resume key. `AvisoClient::watch()` increments the counter; the
    /// supervisor decrements on every exit path. When the prior count is
    /// greater than zero on increment, the client emits a `WARN` log
    /// signalling that two or more concurrent watches share the same
    /// checkpoint slot and will interleave commits.
    pub(super) active_resume_keys: ActiveResumeKeys,
    /// Snapshot of the [`AvisoClientBuilder::danger_accept_invalid_certs`]
    /// setting. Surfaced via [`AvisoClient::danger_accept_invalid_certs`]
    /// so a downstream binary can emit a session-level `WARN` log when
    /// running in insecure-TLS mode.
    pub(super) danger_accept_invalid_certs: bool,
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

    /// Returns whether this client was built with TLS certificate
    /// validation disabled via
    /// [`AvisoClientBuilder::danger_accept_invalid_certs`].
    ///
    /// Exposed so a downstream binary (the `aviso` CLI in particular)
    /// can emit a session-level `WARN` log on startup when the client is
    /// running in insecure-TLS mode. The library itself never logs this
    /// per request because it has no session-level emission seam; the
    /// downstream binary owns the WARN cadence.
    #[must_use]
    pub fn danger_accept_invalid_certs(&self) -> bool {
        self.danger_accept_invalid_certs
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
