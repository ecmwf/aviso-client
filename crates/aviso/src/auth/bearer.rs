//! HTTP Bearer-token authentication provider.

use reqwest::header::HeaderValue;

use crate::ClientError;
use crate::auth::AuthProvider;

/// Bearer-token credentials. Renders as `Bearer <token>`.
///
/// The token is redacted in the `Debug` impl so configuration dumps never leak it.
#[derive(Clone)]
pub struct Bearer {
    token: String,
}

impl Bearer {
    /// Builds a [`Bearer`] provider from a raw token string.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] if `token` is empty. Construction time is the right place
    /// to fail: an empty token would otherwise produce a `Bearer ` header that the server would
    /// reject as 401 with no hint that the client built it wrong.
    pub fn new(token: impl Into<String>) -> crate::Result<Self> {
        let token = token.into();
        if token.is_empty() {
            return Err(ClientError::Config("Bearer token must not be empty".into()));
        }
        Ok(Self { token })
    }
}

impl std::fmt::Debug for Bearer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bearer")
            .field("token", &"<redacted>")
            .finish()
    }
}

#[async_trait::async_trait]
impl AuthProvider for Bearer {
    async fn authorization_header(&self) -> crate::Result<HeaderValue> {
        let header = format!("Bearer {}", self.token);
        let mut value = HeaderValue::from_str(&header)
            .map_err(|e| ClientError::Auth(format!("invalid Bearer header value: {e}")))?;
        // Mark as sensitive so reqwest, hyper, and any downstream debug/log path redacts the
        // header value (D8: "Token contents are never logged").
        value.set_sensitive(true);
        Ok(value)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on a constructor success is the expected diagnostic"
)]
mod tests {
    use super::{AuthProvider, Bearer};

    #[tokio::test]
    async fn header_renders_as_bearer_prefix_plus_token() {
        let bearer = Bearer::new("opaque-jwt-here").unwrap();
        let header = bearer.authorization_header().await.unwrap();
        assert_eq!(header, "Bearer opaque-jwt-here");
    }

    #[tokio::test]
    async fn header_is_marked_sensitive() {
        let bearer = Bearer::new("opaque-jwt-here").unwrap();
        let header = bearer.authorization_header().await.unwrap();
        assert!(
            header.is_sensitive(),
            "Bearer header must be sensitive so downstream log paths redact it"
        );
    }

    #[test]
    fn new_rejects_empty_token() {
        let err = Bearer::new("").unwrap_err();
        assert!(matches!(err, crate::ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn debug_redacts_the_token() {
        let bearer = Bearer::new("super-secret-jwt").unwrap();
        let s = format!("{bearer:?}");
        assert!(
            !s.contains("super-secret-jwt"),
            "token must be redacted: {s}"
        );
        assert!(s.contains("<redacted>"), "redaction marker missing: {s}");
    }
}
