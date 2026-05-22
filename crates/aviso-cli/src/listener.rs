//! Listener spawn task for `aviso listen` and `aviso replay`.
//!
//! Each resolved [`crate::config::ListenerSpec`] becomes one
//! [`tokio::task`] that drains a [`aviso::watch::NotificationStream`]
//! until end-of-stream, listener-level error, or cancellation. The
//! task shares an `Arc<AvisoClient>` with siblings so one task
//! ending does not drop the last client clone and trigger the
//! parent-drop cascade for the remaining listeners.

use std::sync::Arc;

use anyhow::Result;
use aviso::watch::{ResumeStart, WatchRequest};
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
    let triggers: Vec<aviso::watch::Trigger> = spec
        .triggers
        .iter()
        .cloned()
        .map(aviso::watch::TriggerConfig::into_trigger)
        .collect();
    Ok(req.with_triggers(triggers))
}

/// Builds a [`WatchRequest`] configured for replay-only delivery
/// from the supplied cursor.
pub(crate) fn build_replay_request(spec: &ListenerSpec, cursor: ResumeStart) -> WatchRequest {
    let req = WatchRequest::replay_only(spec.event.clone(), cursor);
    let req = req.with_filter(spec.identifiers.clone().into_iter().collect());
    let triggers: Vec<aviso::watch::Trigger> = spec
        .triggers
        .iter()
        .cloned()
        .map(aviso::watch::TriggerConfig::into_trigger)
        .collect();
    req.with_triggers(triggers)
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
pub(crate) async fn spawn_listener_drain(
    client: Arc<AvisoClient>,
    request: WatchRequest,
    mut cancel: watch::Receiver<bool>,
    listener_name: String,
    event_type: String,
) -> Result<(), ClientError> {
    let mut stream = client.watch(request)?;
    loop {
        tokio::select! {
            biased;
            _ = cancel.changed() => {
                tracing::debug!(
                    event.name = "cli.listener.cancelled",
                    listener_name = %listener_name,
                    event_type = %event_type,
                    "listener received cancellation signal; draining and exiting"
                );
                return Ok(());
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
                        return Err(e);
                    }
                    None => {
                        tracing::debug!(
                            event.name = "cli.listener.end_of_stream",
                            listener_name = %listener_name,
                            event_type = %event_type,
                            "listener stream ended cleanly"
                        );
                        return Ok(());
                    }
                }
            }
        }
    }
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
