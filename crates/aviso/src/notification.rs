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

use serde::Serialize;
use serde_json::Value;

use crate::ClientError;

/// A notification to publish via the server.
#[derive(Debug, Clone, Serialize)]
pub struct NotificationRequest {
    /// Event type, matching a schema configured on the server (for example `mars`).
    pub event_type: String,

    /// Identifier key/value pairs, per the schema for `event_type`.
    pub identifier: BTreeMap<String, String>,

    /// Optional free-form payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// A notification received from the server.
///
/// Constructed from the wire-format `CloudEvent` envelope, which is hidden per D9. New envelope
/// fields will appear here as the streaming surface lands; the type is `#[non_exhaustive]` so
/// additions do not break downstream code.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// let (event_type, sequence) = parse_cloudevent_id("mars@42").unwrap();
/// assert_eq!(event_type, "mars");
/// assert_eq!(sequence, 42);
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
#[allow(clippy::unwrap_used, reason = "test code: panic-on-unwrap is the expected diagnostic")]
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
}
