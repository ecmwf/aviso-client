// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] if `user` is empty or contains `:`. The colon ban comes
    /// from RFC 7617 (the colon is the user/password separator and would silently shift bytes
    /// between fields). An empty password is allowed because RFC 7617 permits it.
    pub fn new(user: impl Into<String>, pass: impl Into<String>) -> crate::Result<Self> {
        let user = user.into();
        if user.is_empty() {
            return Err(ClientError::Config(
                "Basic username must not be empty".into(),
            ));
        }
        if user.contains(':') {
            return Err(ClientError::Config(
                "Basic username must not contain ':' per RFC 7617".into(),
            ));
        }
        Ok(Self {
            user,
            pass: pass.into(),
        })
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
        // user is guaranteed non-empty and ':'-free by Basic::new; that invariant lets us format
        // the credentials directly without re-validating on every request.
        let credentials = format!("{}:{}", self.user, self.pass);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
        let header = format!("Basic {encoded}");
        let mut value = HeaderValue::from_str(&header)
            .map_err(|e| ClientError::Auth(format!("invalid Basic header value: {e}")))?;
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
    use super::{AuthProvider, Basic};

    #[tokio::test]
    async fn header_encodes_user_and_password_in_base64() {
        let basic = Basic::new("alice", "wonderland").unwrap();
        let header = basic.authorization_header().await.unwrap();
        // base64("alice:wonderland") = "YWxpY2U6d29uZGVybGFuZA=="
        assert_eq!(header, "Basic YWxpY2U6d29uZGVybGFuZA==");
    }

    #[tokio::test]
    async fn header_is_marked_sensitive() {
        let basic = Basic::new("alice", "wonderland").unwrap();
        let header = basic.authorization_header().await.unwrap();
        assert!(
            header.is_sensitive(),
            "Basic header must be sensitive so downstream log paths redact it"
        );
    }

    #[test]
    fn new_rejects_empty_username() {
        let err = Basic::new("", "pw").unwrap_err();
        assert!(matches!(err, crate::ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn new_rejects_username_with_colon_per_rfc_7617() {
        let err = Basic::new("alice:bob", "pw").unwrap_err();
        assert!(matches!(err, crate::ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn debug_redacts_the_password() {
        let basic = Basic::new("alice", "wonderland").unwrap();
        let s = format!("{basic:?}");
        assert!(s.contains("alice"), "user should be visible: {s}");
        assert!(!s.contains("wonderland"), "password must be redacted: {s}");
        assert!(s.contains("<redacted>"), "redaction marker missing: {s}");
    }
}
