//! HTTP Basic authentication provider.

use base64::Engine;
use reqwest::header::HeaderValue;

use crate::ClientError;
use crate::auth::AuthProvider;

/// Basic authentication credentials. Renders as `Basic <base64(user:pass)>`.
///
/// The password is redacted in the `Debug` impl so configuration dumps never leak it.
#[derive(Clone)]
pub struct Basic {
    user: String,
    pass: String,
}

impl Basic {
    /// Builds a [`Basic`] provider from a username and password pair.
    pub fn new(user: impl Into<String>, pass: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            pass: pass.into(),
        }
    }
}

impl std::fmt::Debug for Basic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Basic")
            .field("user", &self.user)
            .field("pass", &"<redacted>")
            .finish()
    }
}

#[async_trait::async_trait]
impl AuthProvider for Basic {
    async fn authorization_header(&self) -> crate::Result<HeaderValue> {
        let credentials = format!("{}:{}", self.user, self.pass);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
        let header = format!("Basic {encoded}");
        HeaderValue::from_str(&header)
            .map_err(|e| ClientError::Auth(format!("invalid Basic header value: {e}")))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on a constructor success is the expected diagnostic"
)]
mod tests {
    use super::{AuthProvider, Basic};

    #[tokio::test]
    async fn header_encodes_user_and_password_in_base64() {
        let basic = Basic::new("alice", "wonderland");
        let header = basic.authorization_header().await.unwrap();
        // base64("alice:wonderland") = "YWxpY2U6d29uZGVybGFuZA=="
        assert_eq!(header, "Basic YWxpY2U6d29uZGVybGFuZA==");
    }

    #[test]
    fn debug_redacts_the_password() {
        let basic = Basic::new("alice", "wonderland");
        let s = format!("{basic:?}");
        assert!(s.contains("alice"), "user should be visible: {s}");
        assert!(!s.contains("wonderland"), "password must be redacted: {s}");
        assert!(s.contains("<redacted>"), "redaction marker missing: {s}");
    }
}
