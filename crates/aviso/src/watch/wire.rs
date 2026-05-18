//! Private wire-format types for the watch endpoint.
//!
//! These types model the JSON shapes the server exchanges over the watch and
//! replay HTTP endpoints. They are deliberately separate from the public
//! [`super::WatchRequest`] and [`crate::Notification`] types so the public
//! surface stays stable while the wire format evolves.
//!
//! Verified against the `aviso-server` source (`src/types/request.rs`,
//! `src/sse/types.rs`, `src/sse/helpers.rs`, `src/cloudevents/mod.rs`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ResumeStart, WatchRequest};
use crate::ClientError;

/// Body for `POST /api/v1/watch` and `POST /api/v1/replay`.
///
/// `from_id` is a JSON string on the wire (the server parses it back to
/// `u64`); `from_date` is a free-form date string. Server-side validation
/// rejects requests that set both. `event_type` and `identifier` are always
/// present even when the identifier is empty.
#[derive(Debug, Serialize)]
pub(crate) struct WireWatchRequest<'a> {
    pub(crate) event_type: &'a str,
    pub(crate) identifier: &'a BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from_date: Option<&'a str>,
}

impl<'a> WireWatchRequest<'a> {
    /// Build a wire request from the public [`WatchRequest`].
    ///
    /// `ResumeStart::AfterSequence(n)` serialises as `from_id = (n + 1).to_string()`:
    /// the client has already committed everything up to and including `n`,
    /// so the next event to fetch is `n + 1`. Saturation at `u64::MAX` is
    /// rejected with [`ClientError::Config`] rather than wrapping silently.
    pub(crate) fn from_public(req: &'a WatchRequest) -> Result<Self, ClientError> {
        let (from_id, from_date) = match req.from() {
            None => (None, None),
            Some(ResumeStart::AfterSequence(n)) => {
                let next = n.checked_add(1).ok_or_else(|| {
                    ClientError::Config(format!(
                        "from_id overflow: AfterSequence({n}) cannot advance past u64::MAX"
                    ))
                })?;
                (Some(next.to_string()), None)
            }
            Some(ResumeStart::Date(s)) => (None, Some(s.as_str())),
        };
        Ok(WireWatchRequest {
            event_type: req.event_type(),
            identifier: req.filter(),
            from_id,
            from_date,
        })
    }
}

/// `CloudEvent` envelope received over the watch stream.
///
/// The full envelope is documented at
/// `aviso-server/src/cloudevents/mod.rs`; this client decodes only the
/// fields it needs (`id` and `data`) and lets serde discard the rest (per
/// the `CloudEvent`-envelope-hidden ADR).
#[derive(Debug, Deserialize)]
pub(crate) struct WireCloudEvent {
    pub(crate) id: String,
    pub(crate) data: WireCloudEventData,
}

/// Inner `data` object of a `CloudEvent`. `identifier` keys are the schema
/// identifiers reconstructed by the server from the notification topic;
/// values are strings on the read side (in contrast to the request body
/// where values may be JSON constraint objects). `payload` defaults to JSON
/// `null` when absent.
#[derive(Debug, Deserialize)]
pub(crate) struct WireCloudEventData {
    pub(crate) identifier: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) payload: serde_json::Value,
}

/// `live-notification` SSE event with `data.type = "connection_established"`.
///
/// Carries the connection-level metadata the server announces at the head
/// of the stream. Only the `type` discriminant is consumed here; the
/// supervisor treats the rest as observability fodder.
#[derive(Debug, Deserialize)]
pub(crate) struct WireConnectionEstablished {
    #[serde(rename = "type")]
    #[allow(
        dead_code,
        reason = "the supervisor branches on the top-level JSON `type` value before deserialising; this field exists so a future strict-mode check or a unit test can validate the payload shape end-to-end"
    )]
    pub(crate) tag: String,
}

/// `connection-closing` SSE event payload.
///
/// `reason` is one of `server_shutdown`, `max_duration_reached`,
/// `end_of_stream`. Any other value is a wire-protocol error.
/// `request_id` is captured so a fatal reason can be correlated in
/// `ClientError::StreamProtocol`.
#[derive(Debug, Deserialize)]
pub(crate) struct WireConnectionClosing {
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) request_id: Option<String>,
}

/// `replay-control` SSE event payload.
///
/// The discriminant is at `type`; known values are `replay_started`,
/// `replay_completed`, `notification_replay_limit_reached`. Unknown values
/// are ignored by the supervisor (a TRACE log; never fatal).
#[derive(Debug, Deserialize)]
pub(crate) struct WireReplayControl {
    #[serde(rename = "type")]
    pub(crate) tag: String,
}

/// `error` SSE event payload.
///
/// Both `message` and `error` are best-effort: the server populates both in
/// current versions, but the client decodes them as optional so a payload
/// that drops one field still produces a typed `StreamProtocol` error
/// rather than a `Decode` failure.
#[derive(Debug, Deserialize)]
pub(crate) struct WireErrorEvent {
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) request_id: Option<String>,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the standard test diagnostic"
)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{
        WatchRequest, WireCloudEvent, WireConnectionClosing, WireConnectionEstablished,
        WireErrorEvent, WireReplayControl, WireWatchRequest,
    };
    use crate::{ClientError, watch::ResumeStart};

    #[test]
    fn from_public_emits_from_id_as_string_with_next_sequence() {
        let req = WatchRequest::watch_from("mars", ResumeStart::AfterSequence(41));
        let wire = WireWatchRequest::from_public(&req).unwrap();
        assert_eq!(wire.from_id.as_deref(), Some("42"));
        assert!(wire.from_date.is_none());
    }

    #[test]
    fn from_public_emits_from_date_verbatim() {
        let req = WatchRequest::watch_from(
            "mars",
            ResumeStart::Date("2026-01-01T00:00:00Z".to_string()),
        );
        let wire = WireWatchRequest::from_public(&req).unwrap();
        assert!(wire.from_id.is_none());
        assert_eq!(wire.from_date, Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn from_public_omits_resume_fields_when_no_from() {
        let req = WatchRequest::watch("mars");
        let wire = WireWatchRequest::from_public(&req).unwrap();
        assert!(wire.from_id.is_none());
        assert!(wire.from_date.is_none());
    }

    #[test]
    fn from_public_rejects_after_sequence_u64_max_with_config_error() {
        let req = WatchRequest::watch_from("mars", ResumeStart::AfterSequence(u64::MAX));
        let err = WireWatchRequest::from_public(&req).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn from_public_serialises_to_expected_wire_shape() {
        let mut filter = BTreeMap::new();
        filter.insert("country".to_string(), json!("UK"));
        let req =
            WatchRequest::watch_from("mars", ResumeStart::AfterSequence(0)).with_filter(filter);
        let wire = WireWatchRequest::from_public(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(
            json,
            json!({
                "event_type": "mars",
                "identifier": { "country": "UK" },
                "from_id": "1",
            })
        );
    }

    #[test]
    fn from_public_omits_payload_field_completely() {
        let req = WatchRequest::watch("mars");
        let wire = WireWatchRequest::from_public(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert!(
            json.get("payload").is_none(),
            "payload must not appear: {json}"
        );
    }

    #[test]
    fn cloud_event_round_trips_with_identifier_and_payload() {
        let raw = json!({
            "id": "mars@42",
            "source": "https://aviso.example",
            "type": "int.ecmwf.aviso.mars",
            "time": "2026-05-17T12:34:56Z",
            "data": {
                "identifier": { "country": "UK", "class": "od" },
                "payload": { "location": "south" }
            }
        });
        let wire: WireCloudEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(wire.id, "mars@42");
        assert_eq!(
            wire.data.identifier.get("country").map(String::as_str),
            Some("UK")
        );
        assert_eq!(wire.data.payload, json!({ "location": "south" }));
    }

    #[test]
    fn cloud_event_defaults_payload_to_null_when_absent() {
        let raw = json!({
            "id": "mars@1",
            "data": { "identifier": {} }
        });
        let wire: WireCloudEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(wire.data.payload, serde_json::Value::Null);
    }

    #[test]
    fn connection_established_parses_with_only_the_tag_field() {
        let raw = json!({
            "type": "connection_established",
            "topic": "mars.live",
            "timestamp": "2026-05-17T00:00:00Z",
            "connection_will_close_in_seconds": 3600u64,
            "request_id": "req-a"
        });
        let wire: WireConnectionEstablished = serde_json::from_value(raw).unwrap();
        assert_eq!(wire.tag, "connection_established");
    }

    #[test]
    fn connection_closing_captures_reason_and_optional_request_id() {
        let raw = json!({
            "reason": "max_duration_reached",
            "timestamp": "2026-05-17T01:00:00Z",
            "message": "Connection reached maximum duration",
            "topic": "mars.live",
            "request_id": "req-b"
        });
        let wire: WireConnectionClosing = serde_json::from_value(raw).unwrap();
        assert_eq!(wire.reason, "max_duration_reached");
        assert_eq!(wire.request_id.as_deref(), Some("req-b"));
    }

    #[test]
    fn connection_closing_request_id_is_optional() {
        let raw = json!({ "reason": "end_of_stream" });
        let wire: WireConnectionClosing = serde_json::from_value(raw).unwrap();
        assert_eq!(wire.reason, "end_of_stream");
        assert!(wire.request_id.is_none());
    }

    #[test]
    fn replay_control_carries_tag_value() {
        for tag in [
            "replay_started",
            "replay_completed",
            "notification_replay_limit_reached",
            "some_future_tag",
        ] {
            let raw = json!({ "type": tag, "topic": "mars.replay" });
            let wire: WireReplayControl = serde_json::from_value(raw).unwrap();
            assert_eq!(wire.tag, tag);
        }
    }

    #[test]
    fn error_event_decodes_all_fields_when_present() {
        let raw = json!({
            "error": "stream_processing_failed",
            "message": "boom",
            "topic": "mars.live",
            "request_id": "req-c"
        });
        let wire: WireErrorEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(wire.error.as_deref(), Some("stream_processing_failed"));
        assert_eq!(wire.message.as_deref(), Some("boom"));
        assert_eq!(wire.request_id.as_deref(), Some("req-c"));
    }

    #[test]
    fn error_event_defaults_all_fields_to_none_when_payload_is_empty() {
        let raw = json!({});
        let wire: WireErrorEvent = serde_json::from_value(raw).unwrap();
        assert!(wire.error.is_none());
        assert!(wire.message.is_none());
        assert!(wire.request_id.is_none());
    }
}
