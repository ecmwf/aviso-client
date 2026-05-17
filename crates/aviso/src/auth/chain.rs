//! Auth provider composition: try each member in order, first success wins.

use std::sync::Arc;

use reqwest::header::HeaderValue;

use crate::ClientError;
use crate::auth::AuthProvider;

/// Composes multiple [`AuthProvider`] implementations. Each one is tried in order; the first
/// successful header is returned. If all members fail, the last error is propagated. An empty
/// chain always errors.
///
/// `Chain` is a *runtime* fallback over already-constructed providers. It does not perform
/// source discovery, because the shipped source providers ([`crate::auth::Env`],
/// [`crate::auth::ConfigFile`]) fail at construction when their input is missing rather than at
/// header-generation time. Build a chain explicitly from the sources you actually have:
///
/// ```ignore
/// use std::sync::Arc;
/// use aviso::auth::{AuthProvider, Chain, ConfigFile, Env};
///
/// let mut providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
/// if let Ok(env) = Env::from_process_env() {
///     providers.push(Arc::new(env));
/// }
/// if let Ok(file) = ConfigFile::from_path("/etc/aviso/auth.yaml") {
///     providers.push(Arc::new(file));
/// }
/// let _chain = Chain::new(providers);
/// ```
#[derive(Debug)]
pub struct Chain {
    providers: Vec<Arc<dyn AuthProvider>>,
}

impl Chain {
    /// Builds a chain from a vector of [`AuthProvider`] handles.
    #[must_use]
    pub fn new(providers: Vec<Arc<dyn AuthProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait::async_trait]
impl AuthProvider for Chain {
    async fn authorization_header(&self) -> crate::Result<HeaderValue> {
        let mut last_err: Option<ClientError> = None;
        for provider in &self.providers {
            match provider.authorization_header().await {
                Ok(header) => return Ok(header),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| ClientError::Auth("empty AuthProvider chain".into())))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on a constructor success is the expected diagnostic"
)]
mod tests {
    use std::sync::Arc;

    use super::{AuthProvider, Chain, ClientError, HeaderValue};
    use crate::auth::{Basic, Bearer};

    #[derive(Debug)]
    struct AlwaysFails(&'static str);

    #[async_trait::async_trait]
    impl AuthProvider for AlwaysFails {
        async fn authorization_header(&self) -> crate::Result<HeaderValue> {
            Err(ClientError::Auth(self.0.into()))
        }
    }

    #[tokio::test]
    async fn returns_first_successful_header() {
        let chain = Chain::new(vec![
            Arc::new(AlwaysFails("first-fail")),
            Arc::new(Bearer::new("good-token").unwrap()),
            Arc::new(Basic::new("never", "used").unwrap()),
        ]);
        let header = chain.authorization_header().await.unwrap();
        assert_eq!(header, "Bearer good-token");
    }

    #[tokio::test]
    async fn propagates_last_error_when_all_fail() {
        let chain = Chain::new(vec![
            Arc::new(AlwaysFails("first")),
            Arc::new(AlwaysFails("second")),
            Arc::new(AlwaysFails("last")),
        ]);
        let err = chain.authorization_header().await.unwrap_err();
        assert!(
            matches!(&err, ClientError::Auth(msg) if msg == "last"),
            "expected Auth(\"last\"), got {err:?}"
        );
    }

    #[tokio::test]
    async fn empty_chain_errors() {
        let chain = Chain::new(vec![]);
        let err = chain.authorization_header().await.unwrap_err();
        assert!(matches!(err, ClientError::Auth(_)), "got {err:?}");
    }
}
