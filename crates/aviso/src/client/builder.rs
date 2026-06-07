use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client as HttpClient;
use url::Url;

use super::{AvisoClient, DropGuard, RefreshCoordinator};
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
    extra_root_certs: Vec<reqwest::Certificate>,
    danger_accept_invalid_certs: bool,
    flush_cursor_on_exit: bool,
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
            .field("extra_root_certs_count", &self.extra_root_certs.len())
            .field(
                "danger_accept_invalid_certs",
                &self.danger_accept_invalid_certs,
            )
            .field("flush_cursor_on_exit", &self.flush_cursor_on_exit)
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

    /// Adds an additional root certificate that the HTTP client trusts on top of the system
    /// roots. Repeatable; call once per certificate.
    ///
    /// Use this when the `aviso-server` is fronted by a TLS endpoint whose certificate is
    /// signed by an internal CA not in the system trust store (private deployments behind
    /// corporate roots, self-hosted clusters with their own ACME setup, and similar). Each
    /// certificate flows into [`reqwest::ClientBuilder::add_root_certificate`] inside
    /// [`Self::build`]. The system root store stays in effect; this only adds, never
    /// replaces.
    ///
    /// Pair with [`Self::danger_accept_invalid_certs`] only for the narrowest dev
    /// scenarios; the right production move is always to install the real CA via
    /// [`Self::ca_bundle`].
    pub fn ca_bundle(mut self, certificate: reqwest::Certificate) -> Self {
        self.extra_root_certs.push(certificate);
        self
    }

    /// Disables TLS certificate validation entirely. Insecure by design.
    ///
    /// Intended for short-lived dev work against an `aviso-server` that serves a self-signed
    /// certificate, when shipping the cert via [`Self::ca_bundle`] is not practical. Sets
    /// [`reqwest::ClientBuilder::danger_accept_invalid_certs`] to the supplied flag inside
    /// [`Self::build`].
    ///
    /// The flag is also surfaced through [`AvisoClient::danger_accept_invalid_certs`] so a
    /// downstream binary (the CLI in particular) can emit a startup `WARN` log when the
    /// client is built in insecure mode. The library itself does not log per-request because
    /// the lib has no session-level emission seam; the WARN is the binary's responsibility.
    ///
    /// Default `false`. Never set in production.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.danger_accept_invalid_certs = accept;
        self
    }

    /// Opt into flushing the supervisor's in-memory `pending_commit` cursor
    /// to the configured [`StateStore`] when the watch supervisor exits.
    ///
    /// The supervisor's default contract is **commit-on-next-send**: a
    /// notification `N` is persisted to the store only when `N+1` is
    /// about to be delivered, so pulling `N+1` implies `N` is durable.
    /// This preserves at-least-once on a crash: if the consumer dies
    /// after receiving `N` but before processing, the next run resumes
    /// at `N` and re-delivers it.
    ///
    /// The cost is that the LAST notification of every session stays
    /// uncommitted (no `N+1` ever arrives to promote it), so the next
    /// run sees it again. For an interactive operator who has already
    /// observed `N` on their terminal and presses Ctrl+C, that redelivery
    /// is noise.
    ///
    /// When this flag is `true`, the supervisor performs one final
    /// `store.put(pending_commit)` after its reconnect loop exits (for
    /// any reason: cancel signal, fatal error, natural terminal state).
    /// `pending_commit` reflects the LAST notification successfully
    /// delivered through the user-facing channel, so persisting it on
    /// exit is strictly safe: we never claim to have committed more
    /// than was actually sent.
    ///
    /// At-least-once is preserved for **hard** failures (panic, OOM,
    /// SIGKILL) which skip the post-loop flush entirely; only graceful
    /// supervisor exit triggers it.
    ///
    /// Default `false`, preserving the existing contract for library
    /// users that rely on at-least-once redelivery across graceful
    /// restarts. The `aviso` CLI sets this to `true` for `aviso listen`
    /// so operators do not see the same notification on every restart.
    pub fn flush_cursor_on_exit(mut self, flush: bool) -> Self {
        self.flush_cursor_on_exit = flush;
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
        for cert in self.extra_root_certs {
            http_builder = http_builder.add_root_certificate(cert);
        }
        if self.danger_accept_invalid_certs {
            http_builder = http_builder.danger_accept_invalid_certs(true);
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
            refresh_coordinator: Arc::new(RefreshCoordinator::default()),
            parent_drop,
            heartbeat_interval,
            state_store: self.state_store,
            active_resume_keys: Arc::new(Mutex::new(HashMap::new())),
            danger_accept_invalid_certs: self.danger_accept_invalid_certs,
            flush_cursor_on_exit: self.flush_cursor_on_exit,
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
    reason = "test code: unwrap/expect on constructor success is the expected diagnostic"
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

    #[test]
    fn builder_danger_accept_invalid_certs_defaults_false() {
        let client = AvisoClient::builder()
            .base_url("https://localhost:8000")
            .build()
            .unwrap();
        assert!(!client.danger_accept_invalid_certs());
    }

    #[test]
    fn builder_danger_accept_invalid_certs_setter_propagates_to_client() {
        let client = AvisoClient::builder()
            .base_url("https://localhost:8000")
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        assert!(client.danger_accept_invalid_certs());
    }

    /// Self-signed X.509 CA certificate pinned for the TLS-knob tests.
    /// Generated once with
    /// `openssl req -x509 -newkey rsa:2048 -days 36500 -nodes -subj '/CN=aviso-test' -keyout /dev/null -out cert.pem`
    /// and committed verbatim so the test is hermetic: no openssl
    /// invocation at test time, no temp files, no network. Expires in
    /// year 2126.
    const TEST_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIDDTCCAfWgAwIBAgIUOoEsjJSbNYUFzrZXLulyRChR/XEwDQYJKoZIhvcNAQEL\n\
BQAwFTETMBEGA1UEAwwKYXZpc28tdGVzdDAgFw0yNjA1MjExMDU2MTJaGA8yMTI2\n\
MDQyNzEwNTYxMlowFTETMBEGA1UEAwwKYXZpc28tdGVzdDCCASIwDQYJKoZIhvcN\n\
AQEBBQADggEPADCCAQoCggEBAKvtdr6hpcYQ5R7uHt42S95WQqJn/mm6nJxNyM51\n\
4ELO2MZ7X9Vgvy2aVPHsqDV5vHGzZF0F7F+FLA664HAsPnaaghjBnKSW7s4arUb8\n\
4k0RHUi8sivBxYqr5uGbp8uCcas29icFyznaBWELdPmfUFOhhq/BceSmucCoNg0J\n\
pUxsjqRKtfXpWFI4bpaEmKkNneSYneCqkyWBzy+1DxkYE/yY6vkQqmSgb9gjqq1o\n\
WPPyJSw0yyC/jKTp9L0Nz6l7Tn2gdEHDZ9j1nsFy9DD2ZNQ9qlY8fg497gXoa1Mg\n\
Unxhv9usMD6EWWA8yezRxVMcTOEWT9miGEt+Tj6iGLCtXfcCAwEAAaNTMFEwHQYD\n\
VR0OBBYEFGJb4ns++TufwOE+Cbb0VqZMrO7xMB8GA1UdIwQYMBaAFGJb4ns++Tuf\n\
wOE+Cbb0VqZMrO7xMA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEB\n\
AJIsFiiJtf425jlvJxXBsYl8AyiQopvs04K1JpfGpIOsKQxKnOZZzSfUrObQAvjr\n\
IMZEksfPfwOJN4LtPjqzFEO3TqDbWq7bfbzd+pPRh36VceznesuDnBA+z1vNKKH+\n\
8naFx24zL9itWLt9Is/6AFbRfbdYsDExpisLhr4XIQblGPFneq4Bkh9l7szKuMts\n\
WH7j++yZ8PoisM0X0wPuCykZiIXTpdzd3tOkz2KYR7sgvoSugQCN+aYPns2DnXj7\n\
++9qepJtLMoAvtOkutza7a0JuMTkKbnOCiyZELeQq6hHpJuoI2T5lugdanmWkUIF\n\
62aTjKqXhHyepRlFSTwwEAk=\n\
-----END CERTIFICATE-----\n";

    #[test]
    fn builder_ca_bundle_accepts_certificate_built_from_pem() {
        let cert = reqwest::Certificate::from_pem(TEST_CA_PEM)
            .expect("the pinned PEM block parses as a single X.509 certificate");
        let client = AvisoClient::builder()
            .base_url("https://localhost:8000")
            .ca_bundle(cert)
            .build()
            .unwrap();
        assert!(!client.danger_accept_invalid_certs());
    }

    #[test]
    fn builder_ca_bundle_repeatable_succeeds_with_two_certificates() {
        let cert_a =
            reqwest::Certificate::from_pem(TEST_CA_PEM).expect("PEM parses on first construction");
        let cert_b =
            reqwest::Certificate::from_pem(TEST_CA_PEM).expect("PEM parses on second construction");
        let client = AvisoClient::builder()
            .base_url("https://localhost:8000")
            .ca_bundle(cert_a)
            .ca_bundle(cert_b)
            .build()
            .unwrap();
        assert!(!client.danger_accept_invalid_certs());
    }
}
