//! Notification types and `CloudEvent` id parsing.
//!
//! The aviso server speaks `CloudEvents` on the wire; client users see [`Notification`]
//! (received) and [`NotificationRequest`] (sent). The `CloudEvent` envelope itself is
//! intentionally hidden per D9.
//!
//! The `CloudEvent` `id` field is structured as `<event_type>@<sequence>` and is parsed via
//! [`parse_cloudevent_id`]. A malformed id is a terminal [`crate::ClientError::MalformedEvent`]
//! to avoid reconnect livelock on a poisoned server stream.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ClientError;

/// A notification to publish via the server.
///
/// Marked `#[non_exhaustive]` so adding fields (for example, a future schema-version selector)
/// is not a breaking change. Downstream code constructs requests via [`Self::new`] and the
/// `with_*` builder methods rather than struct literals.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct NotificationRequest {
    /// Event type, matching a schema configured on the server (for example `mars`).
    pub event_type: String,

    /// Identifier key/value pairs, per the schema for `event_type`.
    pub identifier: BTreeMap<String, String>,

    /// Optional free-form payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// Server response to a successful `POST /api/v1/notification`.
///
/// Mirrors `aviso-server`'s `NotificationResponse` body. The `request_id` matches the
/// `X-Request-ID` header value the server set on the response; clients should log it so support
/// requests can be correlated against server traces.
///
/// Marked `#[non_exhaustive]` because the server may grow the response shape (for example a
/// sequence echo for in-stream visibility); downstream pattern matching stays compatible.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct NotifyResponse {
    /// Server-supplied status string (typically `"success"`).
    pub status: String,
    /// `X-Request-ID`-equivalent value, echoed in the body for correlation.
    pub request_id: String,
    /// Server-supplied timestamp string. The format is RFC 3339 in current `aviso-server`
    /// versions; the client keeps it as a string to avoid pulling in a date-time dependency.
    pub processed_at: String,
}

impl NotificationRequest {
    /// Builds a [`NotificationRequest`] with the required `event_type` and empty defaults for
    /// the other fields. Use [`Self::with_identifier`] and [`Self::with_payload`] to fill in.
    #[must_use]
    pub fn new(event_type: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            identifier: BTreeMap::new(),
            payload: None,
        }
    }

    /// Replaces the identifier map. Builder-style consumes and returns `self`.
    #[must_use]
    pub fn with_identifier(mut self, identifier: BTreeMap<String, String>) -> Self {
        self.identifier = identifier;
        self
    }

    /// Sets the payload to `Some(value)`. Builder-style consumes and returns `self`.
    #[must_use]
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// A notification received from the server.
///
/// Constructed from the wire-format `CloudEvent` envelope, which is hidden per D9. New envelope
/// fields will appear here as the streaming surface lands; the type is `#[non_exhaustive]` so
/// additions do not break downstream code.
///
/// `Serialize` is derived so trigger dispatchers (and any downstream consumer) can render the
/// notification as JSON without bespoke serialisation code. The wire shape on serialisation is
/// the public field layout: `event_type`, `sequence`, `identifier`, `payload`, `request_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Notification {
    /// Schema event-type, parsed from the `CloudEvent` id (the part before `@`).
    pub event_type: String,

    /// Per-stream monotonic sequence, parsed from the `CloudEvent` id (the part after `@`).
    pub sequence: u64,

    /// Identifier key/value pairs as published.
    pub identifier: BTreeMap<String, String>,

    /// Payload as published. JSON `null` is preserved as [`serde_json::Value::Null`].
    pub payload: Value,

    /// Server-supplied `X-Request-ID` if the notification was returned in a direct HTTP response;
    /// `None` when received over the SSE stream.
    pub request_id: Option<String>,
}

/// Parses a `CloudEvent` `id` field of the form `<event_type>@<sequence>` per D9.
///
/// Returns the event type and the sequence number. A missing `@`, an empty event type, or a
/// sequence part that does not parse as a `u64` produces [`ClientError::MalformedEvent`] with the
/// offending id quoted. This is a terminal error: callers should surface it and stop, not
/// reconnect.
///
/// The split is `rsplit_once('@')` so that an event type containing an `@` character still
/// resolves correctly: only the suffix after the last `@` is treated as the sequence.
///
/// # Examples
///
/// ```
/// use aviso::parse_cloudevent_id;
///
/// // Valid input: <event_type>@<sequence>.
/// let (event_type, sequence) = parse_cloudevent_id("mars@42").unwrap();
/// assert_eq!(event_type, "mars");
/// assert_eq!(sequence, 42);
///
/// // Invalid input: no '@' separator, terminal MalformedEvent error.
/// assert!(parse_cloudevent_id("mars").is_err());
///
/// // Invalid input: sequence overflows u64, terminal MalformedEvent error.
/// assert!(parse_cloudevent_id("mars@18446744073709551616").is_err());
/// ```
///
/// # Errors
///
/// Returns [`ClientError::MalformedEvent`] when the input does not match the expected shape.
pub fn parse_cloudevent_id(id: &str) -> crate::Result<(String, u64)> {
    let (event_type, sequence_str) = id
        .rsplit_once('@')
        .ok_or_else(|| ClientError::MalformedEvent(format!("missing '@' separator: {id:?}")))?;

    if event_type.is_empty() {
        return Err(ClientError::MalformedEvent(format!(
            "empty event_type: {id:?}"
        )));
    }

    let sequence: u64 = sequence_str
        .parse()
        .map_err(|_| ClientError::MalformedEvent(format!("sequence not a u64: {id:?}")))?;

    Ok((event_type.to_string(), sequence))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: panic-on-unwrap is the expected diagnostic"
)]
mod tests {
    use super::{ClientError, parse_cloudevent_id};

    #[test]
    fn parses_valid_id() {
        let (et, seq) = parse_cloudevent_id("mars@42").unwrap();
        assert_eq!(et, "mars");
        assert_eq!(seq, 42);
    }

    #[test]
    fn rsplit_handles_event_type_with_at_inside() {
        let (et, seq) = parse_cloudevent_id("weird@type@99").unwrap();
        assert_eq!(et, "weird@type");
        assert_eq!(seq, 99);
    }

    #[test]
    fn rejects_missing_separator() {
        let err = parse_cloudevent_id("mars").unwrap_err();
        assert!(matches!(err, ClientError::MalformedEvent(_)));
    }

    #[test]
    fn rejects_empty_event_type() {
        let err = parse_cloudevent_id("@42").unwrap_err();
        assert!(matches!(err, ClientError::MalformedEvent(_)));
    }

    #[test]
    fn rejects_non_numeric_sequence() {
        let err = parse_cloudevent_id("mars@abc").unwrap_err();
        assert!(matches!(err, ClientError::MalformedEvent(_)));
    }

    #[test]
    fn rejects_negative_sequence() {
        let err = parse_cloudevent_id("mars@-1").unwrap_err();
        assert!(matches!(err, ClientError::MalformedEvent(_)));
    }

    #[test]
    fn rejects_empty_string() {
        let err = parse_cloudevent_id("").unwrap_err();
        assert!(matches!(err, ClientError::MalformedEvent(_)));
    }

    #[test]
    fn accepts_max_u64_sequence() {
        let id = format!("mars@{}", u64::MAX);
        let (et, seq) = parse_cloudevent_id(&id).unwrap();
        assert_eq!(et, "mars");
        assert_eq!(seq, u64::MAX);
    }

    #[test]
    fn rejects_sequence_one_past_u64_max() {
        // u64::MAX + 1 cannot be represented; the parse must surface as MalformedEvent rather
        // than wrapping or panicking.
        let err = parse_cloudevent_id("mars@18446744073709551616").unwrap_err();
        assert!(matches!(err, ClientError::MalformedEvent(_)), "got {err:?}");
    }

    mod notification_request {
        use std::collections::BTreeMap;

        use crate::NotificationRequest;

        #[test]
        fn new_creates_request_with_event_type_only() {
            let req = NotificationRequest::new("mars");
            assert_eq!(req.event_type, "mars");
            assert!(req.identifier.is_empty());
            assert!(req.payload.is_none());
        }

        #[test]
        fn builder_methods_set_optional_fields() {
            let mut id = BTreeMap::new();
            id.insert("country".to_string(), "uk".to_string());
            let req = NotificationRequest::new("mars")
                .with_identifier(id.clone())
                .with_payload(serde_json::json!({ "location": "south" }));
            assert_eq!(req.event_type, "mars");
            assert_eq!(req.identifier, id);
            assert_eq!(
                req.payload,
                Some(serde_json::json!({ "location": "south" }))
            );
        }

        #[test]
        fn serializes_to_expected_wire_shape() {
            let mut id = BTreeMap::new();
            id.insert("country".to_string(), "uk".to_string());
            let req = NotificationRequest::new("mars")
                .with_identifier(id)
                .with_payload(serde_json::json!({ "location": "south" }));
            let json = serde_json::to_value(&req).unwrap();
            assert_eq!(
                json,
                serde_json::json!({
                    "event_type": "mars",
                    "identifier": { "country": "uk" },
                    "payload": { "location": "south" },
                })
            );
        }

        #[test]
        fn omits_payload_field_when_none() {
            let req = NotificationRequest::new("mars");
            let json = serde_json::to_value(&req).unwrap();
            assert!(
                json.get("payload").is_none(),
                "payload field must be omitted when None: {json}"
            );
        }
    }

    mod notification_serialize {
        use std::collections::BTreeMap;

        use crate::Notification;

        #[test]
        fn serializes_to_expected_wire_shape_with_all_fields() {
            let mut identifier = BTreeMap::new();
            identifier.insert("country".to_string(), "uk".to_string());
            let notification = Notification {
                event_type: "mars".to_string(),
                sequence: 42,
                identifier,
                payload: serde_json::json!({ "location": "south" }),
                request_id: Some("req-abc".to_string()),
            };
            let json = serde_json::to_value(&notification).unwrap();
            assert_eq!(
                json,
                serde_json::json!({
                    "event_type": "mars",
                    "sequence": 42,
                    "identifier": { "country": "uk" },
                    "payload": { "location": "south" },
                    "request_id": "req-abc",
                })
            );
        }

        #[test]
        fn serializes_null_payload_as_json_null() {
            let notification = Notification {
                event_type: "mars".to_string(),
                sequence: 7,
                identifier: BTreeMap::new(),
                payload: serde_json::Value::Null,
                request_id: None,
            };
            let json = serde_json::to_value(&notification).unwrap();
            assert_eq!(json.get("payload"), Some(&serde_json::Value::Null));
            assert_eq!(json.get("request_id"), Some(&serde_json::Value::Null));
        }
    }
}
