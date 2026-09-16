// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Error type returned by aviso client operations.
//!
//! All fallible operations return [`Result<T>`], the crate-local alias for
//! `std::result::Result<T, ClientError>`. The [`ClientError::request_id`]
//! accessor surfaces the server's `X-Request-ID` header on HTTP error
//! responses, and on stream-protocol errors when the SSE payload carried a
//! `request_id` field, so callers can quote it when reporting issues.

use crate::state::StoreError;
use crate::watch::{GapReason, TriggerError, TriggerKindLabel};

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

    /// Stream validation failed, startup timed out, or a fatal stream frame arrived.
    ///
    /// Surfaced when the server emits an `error` SSE event, when a `connection-closing`
    /// frame carries an unrecognised `reason`, or when any other recognised-shape frame is
    /// fundamentally not implementable by this client. The `message` is taken verbatim from
    /// the server payload when one is provided. `request_id` carries the server-supplied
    /// correlation id when the payload includes one.
    ///
    /// Also covers invalid SSE response headers, invalid opening controls, and expired
    /// opening or initial startup deadlines. A deadline can expire before any HTTP
    /// response arrives. [`ClientError::Http`] instead describes an HTTP status failure.
    #[error("stream protocol error (request_id={request_id:?}): {message}")]
    StreamProtocol {
        /// Human-readable description from the client or server-side payload.
        message: String,
        /// Server-supplied `X-Request-ID`-equivalent value, when present.
        request_id: Option<String>,
    },

    /// Configuration-time error (invalid auth source, missing field, and so on).
    #[error("config: {0}")]
    Config(String),

    /// Persistent state-store operation failed during a watch session.
    ///
    /// Surfaced when [`crate::state::StateStore::get`] or
    /// [`crate::state::StateStore::put`] returns an error while the watch
    /// supervisor is consulting or updating the durable resume cursor.
    /// The supervisor cannot continue at-least-once delivery without a
    /// working store, so this error is terminal: the stream yields `None`
    /// after it.
    ///
    /// The `#[from]` impl preserves the underlying error chain
    /// (`StoreError::Io`, `StoreError::Decode`, and so on) so callers can
    /// classify the root cause via `source()` if they need to.
    #[error("state store: {0}")]
    StateStore(#[from] StoreError),

    /// A required trigger failed after all configured retries.
    ///
    /// Surfaced when a [`crate::watch::Trigger`] configured on a
    /// [`crate::watch::WatchRequest`] with `required: true` (the default)
    /// exhausts its retry budget. Terminal: the stream yields `None`
    /// after this error and the supervisor exits.
    ///
    /// The committed checkpoint stays at the previous notification's
    /// sequence (or remains unset if the failing trigger was on the very
    /// first notification of the session). On next process start the
    /// supervisor re-delivers the notification whose trigger failed.
    ///
    /// `kind` names the trigger for diagnostics; `source` carries the
    /// underlying error (typically a [`crate::watch::TriggerError::Io`]).
    #[error("trigger {kind} failed: {source}")]
    TriggerFailed {
        /// Kind of trigger that failed (echo or log).
        kind: TriggerKindLabel,
        /// Inner error from the trigger dispatcher.
        #[source]
        source: TriggerError,
    },
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
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: expect and panic on unexpected variant are the standard test diagnostics"
)]
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

    #[test]
    fn state_store_from_io_error_preserves_chain() {
        use crate::state::StoreError;
        use std::error::Error as _;
        let io = std::io::Error::other("disk full");
        let store_err: StoreError = io.into();
        let client_err: ClientError = store_err.into();
        let rendered = client_err.to_string();
        assert!(rendered.starts_with("state store:"), "got: {rendered}");
        let source = client_err.source();
        assert!(
            source.is_some(),
            "ClientError::StateStore must expose its inner StoreError via source()"
        );
    }

    #[test]
    fn state_store_variant_has_no_request_id() {
        use crate::state::StoreError;
        let store_err: StoreError = std::io::Error::other("boom").into();
        let err: ClientError = store_err.into();
        assert_eq!(err.request_id(), None);
    }

    #[test]
    fn trigger_failed_display_includes_kind_and_source() {
        use crate::watch::{TriggerError, TriggerKindLabel};
        let inner: TriggerError = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied: /var/log/aviso.log",
        )
        .into();
        let err = ClientError::TriggerFailed {
            kind: TriggerKindLabel::Log {
                path: std::path::PathBuf::from("/var/log/aviso.log"),
            },
            source: inner,
        };
        let rendered = err.to_string();
        assert!(
            rendered.contains("trigger log(/var/log/aviso.log) failed"),
            "got: {rendered}"
        );
        assert!(rendered.contains("permission denied"), "got: {rendered}");
    }

    #[test]
    fn trigger_failed_request_id_is_none() {
        use crate::watch::{TriggerError, TriggerKindLabel};
        let err = ClientError::TriggerFailed {
            kind: TriggerKindLabel::Echo,
            source: TriggerError::Io(std::io::Error::other("broken pipe")),
        };
        assert_eq!(err.request_id(), None);
    }

    #[test]
    fn trigger_failed_source_chain_exposes_inner_io_error() {
        use crate::watch::{TriggerError, TriggerKindLabel};
        use std::error::Error as _;
        let err = ClientError::TriggerFailed {
            kind: TriggerKindLabel::Echo,
            source: TriggerError::Io(std::io::Error::other("disk full")),
        };
        let source = err.source().expect("TriggerFailed must expose source");
        let chain = source.to_string();
        assert!(chain.contains("io:"), "got: {chain}");
    }
}
