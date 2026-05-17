//! Error type returned by aviso client operations.
//!
//! All fallible operations return [`Result<T>`], the crate-local alias for
//! `std::result::Result<T, ClientError>`. The [`ClientError::request_id`]
//! accessor surfaces the server's `X-Request-ID` header on HTTP error
//! responses so callers can quote it when reporting issues.

/// Result alias using [`ClientError`] as the error type.
pub type Result<T> = std::result::Result<T, ClientError>;

/// Errors returned by aviso client operations.
///
/// Variants are added as new features land. The enum is marked
/// `#[non_exhaustive]` so downstream `match` arms must include a wildcard.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// Network-level failure: connect, TLS, DNS, or partial body read.
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// Server responded with a non-success HTTP status. Carries the verbatim
    /// response body and the server's `X-Request-ID` header (if present).
    #[error("http {status} (request_id={request_id:?}): {body}")]
    Http {
        /// HTTP status code from the response.
        status: u16,
        /// Verbatim response body (may be empty).
        body: String,
        /// Server-supplied `X-Request-ID` header, when present.
        request_id: Option<String>,
    },

    /// Authentication setup, token refresh, or auth source resolution failed.
    #[error("auth: {0}")]
    Auth(String),

    /// Failed to decode a response body as JSON.
    #[error("decode: {0}")]
    Decode(#[from] serde_json::Error),

    /// `CloudEvent` envelope contained a malformed event id. Terminal per D9 to avoid reconnect
    /// livelock on a poisoned server stream.
    #[error("malformed CloudEvent id: {0}")]
    MalformedEvent(String),

    /// Configuration-time error (invalid auth source, missing field, and so on).
    #[error("config: {0}")]
    Config(String),
}

impl ClientError {
    /// Returns the server-supplied `X-Request-ID` for [`ClientError::Http`]
    /// errors, or `None` for other variants and for HTTP responses that did
    /// not include the header.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Http { request_id, .. } => request_id.as_deref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClientError;

    #[test]
    fn request_id_is_some_for_http_with_header() {
        let err = ClientError::Http {
            status: 400,
            body: "bad".into(),
            request_id: Some("req-123".into()),
        };
        assert_eq!(err.request_id(), Some("req-123"));
    }

    #[test]
    fn request_id_is_none_for_http_without_header() {
        let err = ClientError::Http {
            status: 500,
            body: "oops".into(),
            request_id: None,
        };
        assert_eq!(err.request_id(), None);
    }

    #[test]
    fn request_id_is_none_for_non_http_variants() {
        let auth_err = ClientError::Auth("no token".into());
        assert_eq!(auth_err.request_id(), None);

        let config_err = ClientError::Config("missing".into());
        assert_eq!(config_err.request_id(), None);
    }
}
