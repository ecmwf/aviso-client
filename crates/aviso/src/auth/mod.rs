//! Authentication providers per D8.
//!
//! [`AuthProvider`] is the async trait the client invokes before each request to obtain the
//! `Authorization` header value. Two providers ship in this module: [`Basic`] and [`Bearer`].
//! Additional providers (env-var, config-file, chained composition) live in sibling modules.
//!
//! Shipped providers redact secrets in their `Debug` output.

mod basic;
mod bearer;

pub use basic::Basic;
pub use bearer::Bearer;

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
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Auth`] when the auth source cannot produce a header (missing
    /// token, refresh failure, encoding error, and so on).
    async fn authorization_header(&self) -> crate::Result<HeaderValue>;
}
