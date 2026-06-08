// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Environment-variable auth provider.
//!
//! Reads credentials once at construction. Bearer is preferred when set; otherwise the
//! username/password pair is used.

use reqwest::header::HeaderValue;

use crate::ClientError;
use crate::auth::{AuthProvider, Basic, Bearer};

/// Environment variable holding a Bearer (JWT or opaque) token. Matches the legacy `pyaviso`
/// convention per D8.
pub const ENV_TOKEN: &str = "AVISO_TOKEN";

/// Environment variable holding the Basic-auth username.
pub const ENV_USERNAME: &str = "AVISO_USERNAME";

/// Environment variable holding the Basic-auth password.
pub const ENV_PASSWORD: &str = "AVISO_PASSWORD";

/// Auth provider that picks credentials up from process environment variables.
///
/// Resolution order at construction:
///
/// 1. If `AVISO_TOKEN` is set and non-empty, use [`Bearer`].
/// 2. Else if `AVISO_USERNAME` and `AVISO_PASSWORD` are both set (username non-empty), use [`Basic`].
/// 3. Else return [`ClientError::Auth`] describing what was missing.
#[derive(Debug)]
pub struct Env {
    inner: EnvSource,
}

#[derive(Debug)]
enum EnvSource {
    Bearer(Bearer),
    Basic(Basic),
}

impl Env {
    /// Reads credentials from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] if any of the credential env vars is set but its value is
    /// not valid UTF-8 (`VarError::NotUnicode`). Returns [`ClientError::Auth`] if no usable
    /// combination of variables is set.
    pub fn from_process_env() -> crate::Result<Self> {
        let bearer = read_env_var(ENV_TOKEN)?;
        let user = read_env_var(ENV_USERNAME)?;
        let pass = read_env_var(ENV_PASSWORD)?;
        Self::from_credentials(bearer.as_deref(), user.as_deref(), pass.as_deref())
    }

    /// Resolves credentials from explicit option triples. This is the testable kernel of
    /// [`Self::from_process_env`].
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Auth`] if neither bearer nor (username + password) is supplied.
    pub fn from_credentials(
        bearer_token: Option<&str>,
        username: Option<&str>,
        password: Option<&str>,
    ) -> crate::Result<Self> {
        if let Some(token) = bearer_token.filter(|t| !t.is_empty()) {
            return Ok(Self {
                inner: EnvSource::Bearer(Bearer::new(token)?),
            });
        }
        match (username, password) {
            (Some(u), Some(p)) if !u.is_empty() => Ok(Self {
                inner: EnvSource::Basic(Basic::new(u, p)?),
            }),
            _ => Err(ClientError::Auth(format!(
                "no credentials available: set {ENV_TOKEN} or both {ENV_USERNAME} and {ENV_PASSWORD}"
            ))),
        }
    }
}

#[async_trait::async_trait]
impl AuthProvider for Env {
    async fn authorization_header(&self) -> crate::Result<HeaderValue> {
        match &self.inner {
            EnvSource::Bearer(b) => b.authorization_header().await,
            EnvSource::Basic(b) => b.authorization_header().await,
        }
    }
}

/// Reads an env var, distinguishing absent (`Ok(None)`) from set-but-not-UTF-8
/// (`Err(ClientError::Config)`). The previous `.ok()` shortcut conflated the two and silently
/// fell back to "missing", which produced misleading auth errors when a user had a non-Unicode
/// env value.
fn read_env_var(name: &str) -> crate::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(ClientError::Config(format!(
            "env var {name} is set but its value is not valid UTF-8"
        ))),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on constructor success is the expected diagnostic"
)]
mod tests {
    use super::{AuthProvider, ClientError, Env};

    #[tokio::test]
    async fn bearer_token_takes_precedence_over_basic() {
        let env = Env::from_credentials(Some("jwt-xyz"), Some("alice"), Some("pw")).unwrap();
        let header = env.authorization_header().await.unwrap();
        assert_eq!(header, "Bearer jwt-xyz");
    }

    #[tokio::test]
    async fn falls_back_to_basic_when_no_bearer() {
        let env = Env::from_credentials(None, Some("alice"), Some("wonderland")).unwrap();
        let header = env.authorization_header().await.unwrap();
        assert_eq!(header, "Basic YWxpY2U6d29uZGVybGFuZA==");
    }

    #[test]
    fn errors_when_nothing_is_set() {
        let err = Env::from_credentials(None, None, None).unwrap_err();
        assert!(matches!(err, ClientError::Auth(_)), "got {err:?}");
    }

    #[test]
    fn errors_when_only_username_set() {
        let err = Env::from_credentials(None, Some("alice"), None).unwrap_err();
        assert!(matches!(err, ClientError::Auth(_)), "got {err:?}");
    }

    #[test]
    fn empty_bearer_token_is_treated_as_unset() {
        let env = Env::from_credentials(Some(""), Some("alice"), Some("pw")).unwrap();
        let header = futures_executor_block_on(env.authorization_header());
        assert!(header.unwrap().to_str().unwrap().starts_with("Basic "));
    }

    fn futures_executor_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }
}
