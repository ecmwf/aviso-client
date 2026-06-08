// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Authentication providers per D8.
//!
//! [`AuthProvider`] is the async trait the client invokes before each request to obtain the
//! `Authorization` header value. Five providers ship: [`Basic`], [`Bearer`], [`Env`],
//! [`ConfigFile`], and [`Chain`] (composition).
//!
//! Shipped providers redact secrets in their `Debug` output.

mod basic;
mod bearer;
mod chain;
mod config_file;
mod env;

pub use basic::Basic;
pub use bearer::Bearer;
pub use chain::Chain;
pub use config_file::ConfigFile;
pub use env::Env;

use reqwest::header::HeaderValue;

/// Async trait implemented by anything that can produce the `Authorization` header.
///
/// Implementations must be `Send + Sync` so the client can share them across tasks, and `Debug`
/// so configuration dumps print without panicking. Implementations are expected to redact secrets
/// in their `Debug` representation; the shipped providers do.
#[async_trait::async_trait]
pub trait AuthProvider: Send + Sync + std::fmt::Debug {
    /// Returns the value for the `Authorization` header on each outbound request.
    ///
    /// Implementations carrying real credentials must mark the returned [`HeaderValue`] as
    /// sensitive via [`HeaderValue::set_sensitive`] so downstream log and debug paths (in
    /// `reqwest`, `hyper`, and elsewhere) redact the value per D8. The shipped providers
    /// (`Basic`, `Bearer`, and the ones that wrap them) do this; custom providers must too.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Auth`] when the auth source cannot produce a header (missing
    /// token, refresh failure, encoding error, and so on).
    async fn authorization_header(&self) -> crate::Result<HeaderValue>;

    /// Refreshes credentials after a `401 Unauthorized` response per D8.
    ///
    /// The aviso client calls this method when an authenticated request comes back with
    /// `401`, then retries the request once. The default implementation is a no-op, which is
    /// correct for static-credential providers like [`Basic`] and [`Bearer`] where a `401` means
    /// the credentials are simply wrong and refreshing them changes nothing.
    ///
    /// When several authenticated requests fail with `401` at once (for example a concurrent
    /// batch from [`crate::AvisoClient::notify_many`]), the client coalesces them into a single
    /// `refresh` call rather than invoking this method once per request. Implementations must
    /// therefore not assume one `refresh` per failed request, and must not issue an authenticated
    /// request through the same client from inside `refresh` (it would deadlock the coalescer).
    ///
    /// Implementations that hold cached tokens (OAuth, OIDC, signed-URL providers) override this
    /// to rotate the cached token. Because the trait is taken by shared reference (so the same
    /// provider can be cloned through `Arc<dyn AuthProvider>` across tasks), refresh
    /// implementations must use interior mutability (`Mutex`, `RwLock`, `tokio::sync::RwLock`,
    /// or an atomic) to publish the new token; the next call to
    /// [`Self::authorization_header`] is expected to see it.
    ///
    /// # Cancel-safety
    ///
    /// The watch supervisor races this future against per-stream drop and parent-client drop via
    /// `tokio::select!`, so a refresh in progress may be dropped before completion. Implementations
    /// MUST publish any updated state via the interior-mutability pattern so a dropped refresh
    /// leaves the cached credential in a consistent state (either the old or the new value, never
    /// a torn intermediate). The shipped providers use atomic-swap; a future custom provider that
    /// violates this contract would cause stale-credential bugs on the next `authorization_header`
    /// call, which the next 401 cycle would naturally repair.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Auth`] when refresh itself fails (network error talking to
    /// the token endpoint, refusal by the identity provider, encoding error). The client surfaces
    /// the error verbatim and does not retry the original request.
    async fn refresh(&self) -> crate::Result<()> {
        Ok(())
    }
}
