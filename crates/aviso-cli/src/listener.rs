// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Listener spawn task for `aviso listen` and `aviso replay`.
//!
//! Each resolved [`crate::config::ListenerSpec`] becomes one
//! [`tokio::task`] that drains a [`aviso::watch::NotificationStream`]
//! until end-of-stream, listener-level error, or cancellation. The
//! task shares an `Arc<AvisoClient>` with siblings so one task
//! ending does not drop the last client clone and trigger the
//! parent-drop cascade for the remaining listeners.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use aviso::watch::{EchoConfig, ResumeStart, TriggerConfig, WatchRequest};
use aviso::{AvisoClient, ClientError};
use tokio::sync::watch;

use crate::config::ListenerSpec;
use crate::exit::usage_error;

/// Builds a [`WatchRequest`] from a parsed listener spec.
///
/// Applies the `event`, `identifiers`, `triggers`, and optional
/// `from_id` / `from_date` fields. Returns a usage error when
/// both `from_id` and `from_date` are set (the lib's
/// `WatchRequest::watch_from` accepts one cursor at a time).
pub(crate) fn build_watch_request(spec: &ListenerSpec) -> Result<WatchRequest> {
    let resume_start = match (spec.from_id, spec.from_date.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(usage_error(format!(
                "listener `{}` has both `from_id:` and `from_date:`; set at most one",
                spec.name.as_deref().unwrap_or(&spec.event),
            )));
        }
        (Some(id), None) => Some(ResumeStart::AfterSequence(id)),
        (None, Some(d)) => Some(ResumeStart::Date(d.to_string())),
        (None, None) => None,
    };

    let req = if let Some(start) = resume_start {
        WatchRequest::watch_from(spec.event.clone(), start)
    } else {
        WatchRequest::watch(spec.event.clone())
    };

    let req = req.with_filter(spec.identifiers.clone().into_iter().collect());
    let triggers = triggers_for_listener(spec);
    Ok(req.with_triggers(triggers))
}

/// Builds a [`WatchRequest`] configured for replay-only delivery
/// from the supplied cursor.
pub(crate) fn build_replay_request(spec: &ListenerSpec, cursor: ResumeStart) -> WatchRequest {
    let req = WatchRequest::replay_only(spec.event.clone(), cursor);
    let req = req.with_filter(spec.identifiers.clone().into_iter().collect());
    let triggers = triggers_for_listener(spec);
    req.with_triggers(triggers)
}

/// Builds a single-listener [`ListenerSpec`] from inline CLI
/// arguments. Used by both `aviso listen --event ... --identifiers
/// ...` and the equivalent `aviso replay` path so the two commands
/// stay in sync on inline ad-hoc listener semantics.
///
/// The returned spec carries:
///
/// - `name = Some("ad-hoc")` so the startup banner and echo
///   leader produce predictable, identifying output regardless of
///   which command invoked the path.
/// - No `from_id` / `from_date` defaults; the operator supplies
///   the cursor via `--from` (mandatory for replay, optional for
///   listen).
/// - A single default echo trigger
///   ([`TriggerConfig::Echo`] built from
///   [`EchoConfig::default`]) so the operator sees notifications
///   on stdout without any extra YAML or flags. To customise
///   triggers (log, command, webhook, teams, post) for an ad-hoc
///   run, use a YAML file with a `listeners:` block instead.
///
/// Errors are wrapped via [`usage_error`] (exit code `2`) when
/// the supplied `identifiers_json` does not parse as a JSON
/// object; the error message names the expected shape
/// (`'{"class":"od"}'`) so the operator sees the fix inline.
pub(crate) fn build_inline_listener_spec(
    event: &str,
    identifiers_json: &str,
) -> Result<ListenerSpec> {
    let identifiers: BTreeMap<String, serde_json::Value> = serde_json::from_str(identifiers_json)
        .map_err(|e| {
            usage_error(format!(
                "parse --identifiers as JSON object: {e}; expected something like '{{\"class\":\"od\"}}'"
            ))
        })?;
    Ok(inline_listener_spec(event, identifiers))
}

/// Builds the default echo listener from an already resolved identifier map.
pub(crate) fn inline_listener_spec(
    event: &str,
    identifiers: BTreeMap<String, serde_json::Value>,
) -> ListenerSpec {
    ListenerSpec {
        name: Some("ad-hoc".into()),
        event: event.to_string(),
        identifiers,
        from_id: None,
        from_date: None,
        triggers: vec![TriggerConfig::Echo(EchoConfig::default())],
    }
}

pub(crate) fn triggers_for_listener(spec: &ListenerSpec) -> Vec<aviso::watch::Trigger> {
    let label = spec.name.as_deref();
    spec.triggers
        .iter()
        .cloned()
        .map(aviso::watch::TriggerConfig::into_trigger)
        .map(|t| match label {
            Some(name) => t.label(name),
            None => t,
        })
        .collect()
}

/// Drives one listener to completion.
///
/// Opens the watch via `client.watch(req)`, then drains the
/// stream in a `select!` against `cancel.changed()`. Returns
/// `Ok(())` on clean end-of-stream OR cancellation; returns
/// `Err(ClientError)` on a terminal stream error.
///
/// The function is `async`; the caller spawns it onto a
/// [`tokio::task::JoinSet`] and supervises completion via
/// `JoinSet::join_next_with_id`.
#[tracing::instrument(skip(client, request, cancel), fields(listener = %listener_name))]
pub(crate) async fn spawn_listener_drain(
    client: Arc<AvisoClient>,
    request: WatchRequest,
    mut cancel: watch::Receiver<bool>,
    listener_name: String,
    event_type: String,
) -> Result<(), ClientError> {
    if *cancel.borrow() {
        return Ok(());
    }
    let mut stream = client.watch(request)?;
    let mut ready = stream.subscribe_ready();
    let mut waiting = true;
    let result = loop {
        tokio::select! {
            biased;
            _ = cancel.changed() => {
                tracing::debug!(
                    event.name = "cli.listener.cancelled",
                    listener_name = %listener_name,
                    event_type = %event_type,
                    "listener received cancellation signal; draining and exiting"
                );
                break Ok(());
            }
            confirmed = async { ready.wait_for(|confirmed| *confirmed).await.is_ok() }, if waiting => {
                waiting = false;
                if confirmed {
                    crate::output::write_stderr_line(&format!("Listening for {listener_name} [{event_type}]. Press Ctrl+C to stop."))
                        .map_err(|error| ClientError::Config(format!("write listener status: {error}")))?;
                }
            }
            item = stream.recv() => {
                match item {
                    Some(Ok(notification)) => {
                        tracing::debug!(
                            event.name = "cli.listener.notification",
                            listener_name = %listener_name,
                            event_type = %notification.event_type,
                            sequence = notification.sequence,
                            "received notification"
                        );
                    }
                    Some(Err(e)) => {
                        break Err(e);
                    }
                    None => {
                        tracing::debug!(
                            event.name = "cli.listener.end_of_stream",
                            listener_name = %listener_name,
                            event_type = %event_type,
                            "listener stream ended cleanly"
                        );
                        break Ok(());
                    }
                }
            }
        }
    };
    stream.close().await;
    result
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic specs is the expected diagnostic"
)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec(from_id: Option<u64>, from_date: Option<&str>) -> ListenerSpec {
        ListenerSpec {
            name: Some("test".into()),
            event: "mars".into(),
            identifiers: BTreeMap::new(),
            from_id,
            from_date: from_date.map(String::from),
            triggers: Vec::new(),
        }
    }

    #[test]
    fn no_cursor_yields_plain_watch_request() {
        let req = build_watch_request(&spec(None, None)).unwrap();
        assert_eq!(req.event_type(), "mars");
    }

    #[test]
    fn from_id_yields_watch_from_after_sequence() {
        let req = build_watch_request(&spec(Some(42), None)).unwrap();
        assert_eq!(req.event_type(), "mars");
    }

    #[test]
    fn from_date_yields_watch_from_date() {
        let req = build_watch_request(&spec(None, Some("2024-01-15T00:00:00.000000Z"))).unwrap();
        assert_eq!(req.event_type(), "mars");
    }

    #[test]
    fn build_inline_listener_spec_with_class_od_identifier_builds_ad_hoc_spec_with_default_echo() {
        let spec = build_inline_listener_spec("mars", r#"{"class":"od"}"#).unwrap();
        assert_eq!(
            spec.name.as_deref(),
            Some("ad-hoc"),
            "inline specs are named `ad-hoc` so the startup banner and echo leader produce predictable, identifying output regardless of whether listen or replay invoked the helper",
        );
        assert_eq!(spec.event, "mars");
        assert_eq!(
            spec.identifiers.get("class").map(serde_json::Value::as_str),
            Some(Some("od")),
            "identifiers JSON `{{\"class\":\"od\"}}` must produce the same BTreeMap entry the YAML deserialiser produces for `identifiers: {{ class: od }}`; otherwise inline and YAML modes would not be substitutable",
        );
        assert!(
            spec.from_id.is_none() && spec.from_date.is_none(),
            "inline specs carry no per-listener cursor; the operator supplies the cursor via --from (optional for listen, mandatory for replay): from_id={:?}, from_date={:?}",
            spec.from_id,
            spec.from_date,
        );
        assert_eq!(
            spec.triggers.len(),
            1,
            "inline specs carry exactly one default echo trigger so the operator sees notifications on stdout without any extra YAML or flags; without this default the inline command silently discards everything and looks broken",
        );
        assert!(
            matches!(spec.triggers.first(), Some(TriggerConfig::Echo(_))),
            "the default trigger MUST be Echo specifically (not Log/Command/Webhook/etc.) because Echo is the only trigger whose default behavior is operator-visible without further configuration",
        );
    }

    #[test]
    fn build_inline_listener_spec_invalid_json_identifiers_returns_usage_error_naming_expected_shape()
     {
        let err = build_inline_listener_spec("mars", "{not valid json").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("parse --identifiers"),
            "error message must name the failing flag so the operator knows which argument to fix; got: {msg}",
        );
        assert!(
            msg.contains(r#"'{"class":"od"}'"#),
            "error message must include a canonical example of valid input so the operator sees the expected shape inline; got: {msg}",
        );
    }

    #[test]
    fn build_inline_listener_spec_with_empty_identifiers_object_is_a_valid_wildcard_listener() {
        let spec = build_inline_listener_spec("mars", "{}").unwrap();
        assert!(
            spec.identifiers.is_empty(),
            "an empty JSON object must produce an empty identifiers map; this is the valid `tail everything for event_type` shape",
        );
    }

    #[test]
    fn build_inline_listener_spec_preserves_point_cloud_array() {
        let spec = build_inline_listener_spec("observations", r#"{"point_cloud":[[46,8],[47,9]]}"#)
            .unwrap();
        assert_eq!(
            spec.identifiers.get("point_cloud"),
            Some(&serde_json::json!([[46, 8], [47, 9]]))
        );
    }

    #[test]
    fn build_inline_listener_spec_propagates_the_event_type_argument_unchanged() {
        for ev in &["mars", "test_polygon", "int.ecmwf.aviso.mars"] {
            let spec = build_inline_listener_spec(ev, "{}").unwrap();
            assert_eq!(
                spec.event, *ev,
                "event_type must round-trip from CLI arg to spec verbatim so the operator sees what they typed; ev={ev}",
            );
        }
    }

    #[test]
    fn both_cursors_set_errors() {
        let err =
            build_watch_request(&spec(Some(42), Some("2024-01-15T00:00:00.000000Z"))).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("from_id") || msg.contains("from_date"),
            "{msg}"
        );
    }
}
