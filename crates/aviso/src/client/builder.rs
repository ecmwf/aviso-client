use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client as HttpClient;
use url::Url;

use super::{AvisoClient, DropGuard};
use crate::ClientError;
use crate::auth::AuthProvider;
use crate::state::StateStore;

/// Builder for [`AvisoClient`].
#[derive(Default)]
#[must_use]
pub struct AvisoClientBuilder {
    base_url: Option<String>,
    auth: Option<Arc<dyn AuthProvider>>,
    timeout: Option<Duration>,
    user_agent: Option<String>,
    heartbeat_interval: Option<Duration>,
    state_store: Option<Arc<dyn StateStore>>,
}

impl std::fmt::Debug for AvisoClientBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvisoClientBuilder")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .field("timeout", &self.timeout)
            .field("user_agent", &self.user_agent)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("state_store", &self.state_store.as_ref().map(|_| "<set>"))
            .finish()
    }
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

    /// Wires a persistent state store for resume across process restarts.
    ///
    /// When set, `AvisoClient::watch()` consults the store at watch
    /// start: if the [`crate::watch::WatchRequest`] has no explicit `from`, the
    /// supervisor reads the stored checkpoint and resumes from
    /// `last_committed_sequence + 1`. An explicit user-supplied `from`
    /// always wins (no second-guessing). After each successful
    /// notification send the supervisor persists the *previous*
    /// notification's sequence (commit-on-next-send semantics): pulling
    /// item N+1 implies item N is durable.
    ///
    /// The store can be the in-process [`crate::state::MemoryStore`], the
    /// on-disk [`crate::state::JsonFileStore`], or a user-supplied
    /// implementation of the [`StateStore`] trait. The watch supervisor
    /// terminates the stream with `ClientError::StateStore` on any
    /// persistence failure; at-least-once delivery requires a working
    /// store, and silent failure would violate the contract.
    pub fn state_store(mut self, store: Arc<dyn StateStore>) -> Self {
        self.state_store = Some(store);
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
            state_store: self.state_store,
            active_resume_keys: Arc::new(Mutex::new(HashMap::new())),
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
    reason = "test code: unwrap on constructor success is the expected diagnostic"
)]
mod tests {
    use super::AvisoClient;

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
}
