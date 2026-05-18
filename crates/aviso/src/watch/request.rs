//! [`WatchRequest`]: caller-facing description of a watch subscription.
//!
//! A `WatchRequest` names the event type to subscribe to, an optional filter
//! over the event identifier, an optional resume position, and the session
//! mode. The supervisor that consumes it (in a follow-up commit) selects the
//! HTTP endpoint from [`WatchMode`] and serialises the rest as the
//! server-side request body.
//!
//! Three constructors make the impossible state ("replay-only without a
//! resume position") unrepresentable by construction: [`Self::watch`] for
//! live-only, [`Self::watch_from`] for historical-then-live, and
//! [`Self::replay_only`] for replay-and-terminate. The pattern mirrors
//! [`crate::watch::WatchState::watch`] and
//! [`crate::watch::WatchState::replay_only`].

use std::collections::BTreeMap;

use super::{ResumeStart, WatchMode};

/// Subscription parameters for a single watch session.
///
/// Fields are private; use the constructors and [`Self::with_filter`] to
/// build, and the accessor methods to read. The type is `#[non_exhaustive]`
/// so future additions cannot become breaking changes for downstream
/// callers.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use serde_json::json;
/// use aviso::watch::{ResumeStart, WatchMode, WatchRequest};
///
/// // Live-only watch.
/// let req = WatchRequest::watch("mars");
/// assert_eq!(req.event_type(), "mars");
/// assert_eq!(req.mode(), WatchMode::Watch);
/// assert!(req.from().is_none());
///
/// // Historical-then-live, resuming after sequence 41.
/// let req = WatchRequest::watch_from("mars", ResumeStart::AfterSequence(41));
/// assert!(matches!(req.from(), Some(ResumeStart::AfterSequence(41))));
///
/// // Replay-only with a JSON-valued spatial filter.
/// let mut filter = BTreeMap::new();
/// filter.insert("country".to_string(), json!("UK"));
/// let req = WatchRequest::replay_only("mars", ResumeStart::AfterSequence(10))
///     .with_filter(filter);
/// assert_eq!(req.mode(), WatchMode::ReplayOnly);
/// assert_eq!(req.filter().len(), 1);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct WatchRequest {
    event_type: String,
    filter: BTreeMap<String, serde_json::Value>,
    from: Option<ResumeStart>,
    mode: WatchMode,
}

impl WatchRequest {
    /// Build a live-only watch on `event_type`.
    ///
    /// The session has no replay backlog: the server starts streaming live
    /// notifications immediately. [`Self::mode`] is [`WatchMode::Watch`] and
    /// [`Self::from`] is `None`.
    #[must_use]
    pub fn watch(event_type: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            filter: BTreeMap::new(),
            from: None,
            mode: WatchMode::Watch,
        }
    }

    /// Build a watch on `event_type` that first replays from `from` and then
    /// continues live.
    ///
    /// The session starts in the replay phase. After the server emits its
    /// `replay_completed` control event, the supervisor transitions to live
    /// streaming. [`Self::mode`] is [`WatchMode::Watch`] and [`Self::from`]
    /// is `Some(from)`.
    #[must_use]
    pub fn watch_from(event_type: impl Into<String>, from: ResumeStart) -> Self {
        Self {
            event_type: event_type.into(),
            filter: BTreeMap::new(),
            from: Some(from),
            mode: WatchMode::Watch,
        }
    }

    /// Build a replay-only watch on `event_type` from `from`.
    ///
    /// The session terminates after the server emits `replay_completed`
    /// followed by `end_of_stream`. [`Self::mode`] is
    /// [`WatchMode::ReplayOnly`] and [`Self::from`] is `Some(from)`.
    ///
    /// `from` is required because a replay-only session without a resume
    /// position has nothing to replay; the natural-termination path needs to
    /// land in the replay phase before the server can signal completion.
    /// This mirrors the constraint already enforced by
    /// [`crate::watch::WatchState::replay_only`].
    #[must_use]
    pub fn replay_only(event_type: impl Into<String>, from: ResumeStart) -> Self {
        Self {
            event_type: event_type.into(),
            filter: BTreeMap::new(),
            from: Some(from),
            mode: WatchMode::ReplayOnly,
        }
    }

    /// Replace the identifier filter.
    ///
    /// Filter values are `serde_json::Value` because the server's wire
    /// contract accepts JSON values, not just strings: spatial filters
    /// (polygon, point) and range constraints are JSON objects. Pass scalar
    /// string filters as `Value::String(...)`.
    ///
    /// This method replaces the entire map (it does not merge); callers that
    /// want incremental population should accumulate their own map and call
    /// `with_filter` once.
    #[must_use]
    pub fn with_filter(mut self, filter: BTreeMap<String, serde_json::Value>) -> Self {
        self.filter = filter;
        self
    }

    /// Borrow the configured event type.
    #[must_use]
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Borrow the configured identifier filter. An empty map means no filter
    /// is applied; every notification on the event type matches.
    #[must_use]
    pub fn filter(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.filter
    }

    /// Borrow the configured resume position, if any.
    #[must_use]
    pub fn from(&self) -> Option<&ResumeStart> {
        self.from.as_ref()
    }

    /// Return the configured session mode.
    #[must_use]
    pub fn mode(&self) -> WatchMode {
        self.mode
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{ResumeStart, WatchMode, WatchRequest};

    #[test]
    fn watch_constructor_defaults_to_live_watch_mode_with_no_resume() {
        let req = WatchRequest::watch("mars");
        assert_eq!(req.event_type(), "mars");
        assert_eq!(req.mode(), WatchMode::Watch);
        assert!(req.from().is_none());
        assert!(req.filter().is_empty());
    }

    #[test]
    fn watch_from_carries_resume_position_and_keeps_watch_mode() {
        let req = WatchRequest::watch_from("mars", ResumeStart::AfterSequence(41));
        assert_eq!(req.event_type(), "mars");
        assert_eq!(req.mode(), WatchMode::Watch);
        assert_eq!(req.from(), Some(&ResumeStart::AfterSequence(41)));
    }

    #[test]
    fn watch_from_accepts_date_resume_position() {
        let req = WatchRequest::watch_from(
            "mars",
            ResumeStart::Date("2026-01-01T00:00:00Z".to_string()),
        );
        assert!(matches!(req.from(), Some(ResumeStart::Date(_))));
    }

    #[test]
    fn replay_only_requires_resume_position_and_sets_replay_only_mode() {
        let req = WatchRequest::replay_only("mars", ResumeStart::AfterSequence(10));
        assert_eq!(req.mode(), WatchMode::ReplayOnly);
        assert_eq!(req.from(), Some(&ResumeStart::AfterSequence(10)));
    }

    #[test]
    fn with_filter_replaces_the_filter_map() {
        let mut first = BTreeMap::new();
        first.insert("country".to_string(), json!("UK"));
        let mut second = BTreeMap::new();
        second.insert("class".to_string(), json!("od"));
        second.insert("stream".to_string(), json!("oper"));

        let req = WatchRequest::watch("mars")
            .with_filter(first)
            .with_filter(second.clone());
        assert_eq!(req.filter(), &second);
    }

    #[test]
    fn with_filter_accepts_json_value_constraint_objects() {
        let mut filter = BTreeMap::new();
        filter.insert("country".to_string(), json!("UK"));
        filter.insert(
            "polygon".to_string(),
            json!({"type": "polygon", "points": [[0, 0], [1, 1]]}),
        );

        let req = WatchRequest::watch("mars").with_filter(filter);
        assert!(req.filter().get("country").is_some());
        assert!(
            req.filter()
                .get("polygon")
                .and_then(|v| v.as_object())
                .is_some()
        );
    }

    #[test]
    fn clone_preserves_all_fields() {
        let req = WatchRequest::replay_only("mars", ResumeStart::AfterSequence(7));
        let cloned = req.clone();
        assert_eq!(cloned.event_type(), req.event_type());
        assert_eq!(cloned.mode(), req.mode());
        assert_eq!(cloned.from(), req.from());
        assert_eq!(cloned.filter(), req.filter());
    }
}
