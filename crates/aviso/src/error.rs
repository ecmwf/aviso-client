//! Error type returned by aviso client operations.
//!
//! All fallible operations return [`Result<T>`], the crate-local alias for
//! `std::result::Result<T, ClientError>`. The [`ClientError::request_id`]
//! accessor surfaces the server's `X-Request-ID` header on HTTP error
//! responses, and on stream-protocol errors when the SSE payload carried a
//! `request_id` field, so callers can quote it when reporting issues.

use crate::watch::GapReason;

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

    /// A gap was detected in the watch stream. The accompanying [`GapReason`] explains why.
    ///
    /// This variant is surfaced on the watch stream when the supervisor detects either a
    /// non-consecutive sequence number on the wire or a server-emitted
    /// `notification_replay_limit_reached` signal. The watch terminates after this error
    /// because continuing past a known gap would silently violate at-least-once delivery.
    #[error("history gap: {reason:?}")]
    HistoryGap {
        /// The kind of gap that was detected.
        reason: GapReason,
    },

    /// A wire-protocol-level fatal condition was observed on the watch stream.
    ///
    /// Surfaced when the server emits an `error` SSE event, when a `connection-closing`
    /// frame carries an unrecognised `reason`, or when any other recognised-shape frame is
    /// fundamentally not implementable by this client. The `message` is taken verbatim from
    /// the server payload when one is provided. `request_id` carries the server-supplied
    /// correlation id when the payload includes one.
    ///
    /// Distinct from [`ClientError::Http`], which describes a non-success HTTP status on the
    /// initial response; `StreamProtocol` is for fatal frames that arrive over a stream that
    /// initially returned 200.
    #[error("stream protocol error (request_id={request_id:?}): {message}")]
    StreamProtocol {
        /// Human-readable description from the server-side payload.
        message: String,
        /// Server-supplied `X-Request-ID`-equivalent value, when present.
        request_id: Option<String>,
    },

    /// Configuration-time error (invalid auth source, missing field, and so on).
    #[error("config: {0}")]
    Config(String),
}

impl ClientError {
    /// Returns the server-supplied request-id correlation for variants that carry one,
    /// or `None` otherwise.
    ///
    /// Currently this is [`ClientError::Http`] and [`ClientError::StreamProtocol`]; the
    /// other variants either represent client-side conditions ([`ClientError::Config`],
    /// [`ClientError::Auth`]) or do not reliably carry a server identifier
    /// ([`ClientError::Transport`], [`ClientError::Decode`], [`ClientError::MalformedEvent`],
    /// [`ClientError::HistoryGap`]).
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Http { request_id, .. } | Self::StreamProtocol { request_id, .. } => {
                request_id.as_deref()
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClientError;
    use crate::watch::GapReason;

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

    #[test]
    fn request_id_is_some_for_stream_protocol_with_correlation() {
        let err = ClientError::StreamProtocol {
            message: "stream_processing_failed".into(),
            request_id: Some("req-xyz".into()),
        };
        assert_eq!(err.request_id(), Some("req-xyz"));
    }

    #[test]
    fn request_id_is_none_for_stream_protocol_without_correlation() {
        let err = ClientError::StreamProtocol {
            message: "stream_processing_failed".into(),
            request_id: None,
        };
        assert_eq!(err.request_id(), None);
    }

    #[test]
    fn history_gap_carries_reason_and_has_no_request_id() {
        let reason = GapReason::SequenceJump {
            expected: 5,
            observed: 7,
        };
        let err = ClientError::HistoryGap { reason };
        assert_eq!(err.request_id(), None);
        let rendered = err.to_string();
        assert!(rendered.contains("SequenceJump"), "got: {rendered}");
    }

    #[test]
    fn stream_protocol_display_includes_message_and_request_id() {
        let err = ClientError::StreamProtocol {
            message: "boom".into(),
            request_id: Some("r-1".into()),
        };
        let rendered = err.to_string();
        assert!(rendered.contains("boom"), "got: {rendered}");
        assert!(rendered.contains("r-1"), "got: {rendered}");
    }
}
