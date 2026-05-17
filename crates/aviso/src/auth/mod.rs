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
    /// Implementations that hold cached tokens (OAuth, OIDC, signed-URL providers) override this
    /// to rotate the cached token. Because the trait is taken by shared reference (so the same
    /// provider can be cloned through `Arc<dyn AuthProvider>` across tasks), refresh
    /// implementations must use interior mutability (`Mutex`, `RwLock`, `tokio::sync::RwLock`,
    /// or an atomic) to publish the new token; the next call to
    /// [`Self::authorization_header`] is expected to see it.
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
