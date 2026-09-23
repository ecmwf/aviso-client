// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
    /// Set when `auth` was found by the credential search rather than named
    /// by the caller. `build` then refuses a plain http address that is not
    /// loopback, whatever order the address and the credential arrived in.
    auth_was_found: Option<crate::auth::CredentialSource>,
    /// Set when the builder came from the resolver, so a missing address can
    /// say where it looked: the config file, and `AVISO_BASE_URL` too when
    /// the resolver read it.
    looked_up: Option<super::resolve::EnvAddress>,
}

impl std::fmt::Debug for AvisoClientBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvisoClientBuilder")
            .field(
                "base_url",
                &self
                    .base_url
                    .as_deref()
                    .map(crate::auth::url_without_userinfo),
            )
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
            .field("auth_was_found", &self.auth_was_found)
            .field("looked_up", &self.looked_up)
            .finish()
    }
}

impl AvisoClientBuilder {
    /// Starts from the settings in the default config file, and finds a
    /// credential the same way the `aviso` binary does.
    ///
    /// Reads `~/.config/aviso/config.yaml`, or the file named in
    /// `AVISO_CLIENT_CONFIG_FILE`, for `base_url`, `timeout`,
    /// `heartbeat_interval` and `tls`. A missing file is fine and sets
    /// nothing; a file that exists but cannot be used is an error. Then the
    /// credential search runs: the environment, the file's `auth:` block, the
    /// credentials file. A credential found that way is not sent to a plain
    /// http address unless it is loopback; [`Self::build`] checks that
    /// against the address the client ends up with, however it was set.
    ///
    /// Every setter still works on the result and replaces what the file
    /// said, so the precedence is code over file with nothing else to learn:
    ///
    /// ```no_run
    /// use aviso::AvisoClient;
    ///
    /// # fn main() -> aviso::Result<()> {
    /// let client = AvisoClient::builder_from_file()?
    ///     .timeout(std::time::Duration::from_secs(10))
    ///     .build()?;
    /// # let _ = client;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when the file exists but cannot be read
    /// or parsed, or a certificate it names cannot be loaded, and
    /// [`ClientError::Auth`] when a credential source is present but
    /// unusable. The plaintext-address refusal is reported by
    /// [`Self::build`], not here.
    pub fn from_file() -> crate::Result<Self> {
        Self::from_resolution(super::resolve::resolve(
            &super::resolve::CodeInputs::default(),
            &crate::auth::DiscoveryPaths::from_env(),
            super::resolve::EnvAddress::Ignore,
        )?)
    }

    /// Like [`Self::from_file`], with two differences: the `AVISO_BASE_URL`
    /// environment variable is consulted for the address, after `inputs`
    /// and before the file, and the values in `inputs` are applied to the
    /// builder, so nothing needs setting afterwards. This is the path a
    /// client built with no arguments takes; see [`super::resolve`] for the
    /// order.
    ///
    /// # Errors
    ///
    /// As [`Self::from_file`].
    pub fn from_environment(inputs: &super::resolve::CodeInputs) -> crate::Result<Self> {
        Self::from_resolution(super::resolve::resolve(
            inputs,
            &crate::auth::DiscoveryPaths::from_env(),
            super::resolve::EnvAddress::Read,
        )?)
    }

    /// A builder carrying everything a [`resolve`](super::resolve::resolve)
    /// call chose: the winning address, timeouts and certificates, and the
    /// credential, whether named in the inputs or found with its source.
    /// Setters called afterwards replace what was chosen. This is how to
    /// build the client a report describes without resolving twice.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when a certificate the file names
    /// cannot be loaded.
    pub fn from_resolution(resolution: super::resolve::Resolution) -> crate::Result<Self> {
        let mut builder = Self {
            looked_up: Some(resolution.env_address),
            ..Self::default()
        };
        if let Some(url) = resolution.raw_base_url {
            builder = builder.base_url(url);
        }
        let report = &resolution.settings;
        if let Some(t) = report.timeout.value {
            builder = builder.timeout(t);
        }
        if let Some(h) = report.heartbeat_interval.value {
            builder = builder.heartbeat_interval(h);
        }
        for path in &resolution.file_settings.ca_bundle {
            builder = builder.ca_bundle(super::settings::read_ca_bundle(path)?);
        }
        if report.danger_accept_invalid_certs.value {
            builder = builder.danger_accept_invalid_certs(true);
        }
        // The address may still change before build, so the plaintext rule is
        // not applied here. Record that the credential was found; build
        // checks it against the address the client will actually use.
        if let Some(found) = resolution.found {
            builder = builder.found_auth(found);
        }
        match resolution.code_auth {
            Some(super::resolve::CodeAuth::Named(provider)) => builder = builder.auth(provider),
            Some(super::resolve::CodeAuth::Anonymous) => builder = builder.anonymous(),
            None => {}
        }
        Ok(builder)
    }

    /// Like [`Self::from_file`], reading a specific file.
    ///
    /// The path must exist: naming a file that is not there is a mistake, not
    /// an empty configuration. Its `auth:` block, rather than the default
    /// file's, takes part in the credential search.
    ///
    /// # Errors
    ///
    /// As [`Self::from_file`], and [`ClientError::Config`] when the path does
    /// not exist.
    pub fn from_file_at(path: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        // A named file must exist; resolve() treats a missing default file as
        // empty, so check first.
        let loaded = super::settings::ClientSettings::read(path.as_ref())?;
        let mut paths = crate::auth::DiscoveryPaths::from_env();
        paths.config_file = Some(loaded.path);
        paths.config_content = Some(loaded.content);
        Self::from_resolution(super::resolve::resolve(
            &super::resolve::CodeInputs::default(),
            &paths,
            super::resolve::EnvAddress::Ignore,
        )?)
    }

    /// The base URL set so far, if any. Useful after [`Self::from_file`],
    /// where the value came from the file rather than from the caller.
    #[must_use]
    pub fn configured_base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// Sets the `aviso-server` base URL. Required.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Sets the auth provider. Optional; the client sends no `Authorization` header when unset,
    /// which is the right configuration for anonymous-access streams on `aviso-server`.
    pub fn auth(mut self, auth: Arc<dyn AuthProvider>) -> Self {
        self.auth = Some(auth);
        self.auth_was_found = None;
        self
    }

    /// Attaches a credential that was found rather than named.
    ///
    /// This is how discovery hands its result to the builder. Unlike
    /// [`Self::auth`], it keeps the record that the credential was found, so
    /// [`Self::build`] refuses to send it to a plain http address that is not
    /// loopback, whatever address the builder ends up with. Callers that
    /// searched for a credential themselves use this rather than `auth`, or
    /// the refusal silently stops applying.
    pub fn found_auth(mut self, found: crate::auth::Discovered) -> Self {
        self.auth_was_found = Some(found.source().clone());
        self.auth = Some(found.into_provider());
        self
    }

    /// Removes any auth provider, so the client sends no `Authorization`
    /// header. This is how a builder from [`Self::from_file`] is made
    /// anonymous after the credential search has attached something.
    pub fn anonymous(mut self) -> Self {
        self.auth = None;
        self.auth_was_found = None;
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
    /// Returns [`crate::ClientError::Config`] when `base_url` is missing, is not a
    /// valid URL, or uses a scheme other than HTTP or HTTPS. Also returned when
    /// the underlying `reqwest::Client` cannot be built.
    ///
    /// Returns [`crate::ClientError::Auth`] when the credential was found by
    /// [`Self::from_file`] rather than named with [`Self::auth`], and
    /// `base_url` is plain http to an address other than loopback.
    pub fn build(self) -> crate::Result<AvisoClient> {
        let looked_up = self.looked_up;
        let raw = self.base_url.ok_or_else(|| {
            ClientError::Config(match looked_up {
                Some(super::resolve::EnvAddress::Read) => {
                    "AvisoClient requires a base_url; none was set in code, and none was \
                     found in AVISO_BASE_URL or the config file"
                        .into()
                }
                Some(super::resolve::EnvAddress::Ignore) => {
                    "AvisoClient requires a base_url; none was set in code, and the \
                     config file has none"
                        .into()
                }
                None => "AvisoClient requires a base_url".into(),
            })
        })?;
        let mut base_url =
            Url::parse(&raw).map_err(|e| ClientError::Config(format!("invalid base_url: {e}")))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(ClientError::Config(
                "base_url must use http or https".into(),
            ));
        }
        if let Some(source) = &self.auth_was_found
            && crate::auth::is_public_plaintext(&raw)
        {
            return Err(ClientError::Auth(format!(
                "refusing to send the credential from the {source} to {}, which is not \
                 https and not a loopback address. Use an https address, or name the \
                 credential with .auth() if you intend to send it in the clear.",
                crate::auth::url_without_userinfo(&raw)
            )));
        }
        if self.auth.is_some() && crate::auth::is_public_plaintext(&raw) {
            // A named credential is the caller's decision, so it is sent.
            // Say so once, the way the CLI does for disabled certificate
            // checks, so a log scraper can flag it.
            tracing::warn!(
                event.name = "client.auth.plaintext",
                base_url = %crate::auth::url_without_userinfo(&raw),
                "sending the credential over plain http to a non-loopback address; \
                 anyone on the path can read it. Use https."
            );
        }
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
    fn builder_debug_strips_userinfo_from_base_url() {
        let builder = AvisoClient::builder().base_url("https://operator:hunter2@aviso.example.org");
        let formatted = format!("{builder:?}");
        assert!(formatted.contains("aviso.example.org"), "got: {formatted}");
        assert!(!formatted.contains("hunter2"), "got: {formatted}");
        assert!(!formatted.contains("operator"), "got: {formatted}");
        // A value that does not parse is not echoed either.
        let builder = AvisoClient::builder().base_url("https://operator:hunter2@");
        let formatted = format!("{builder:?}");
        assert!(!formatted.contains("hunter2"), "got: {formatted}");
    }

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
