// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `aviso replay` subcommand.
//!
//! Resolution mirrors `aviso listen` for positional vs config
//! listener selection, with the additional disambiguation:
//!
//! - `--listener <NAME>` selects that one entry from the resolved
//!   set.
//! - No `--listener` and exactly one resolved entry: that one.
//! - No `--listener` and zero or multiple resolved entries: exit 2
//!   with usage naming the available names.
//! - `--event` with `--identifiers <JSON>` or repeated `--identifier` overrides the
//!   listener spec entirely for an ad-hoc replay (both required or
//!   neither, enforced by clap).
//!
//! `--from <VALUE>` is mandatory and parsed via
//! [`crate::from_value`].

use std::path::PathBuf;

use anyhow::Result;
use aviso::watch::{ResumeStart, Trigger};

use crate::cancel;
use crate::client_builder;
use crate::config::{ListenerSpec, Resolved};
use crate::exit::usage_error;
use crate::from_value;
use crate::listener;
use crate::listener_file;
use crate::output;

/// Runs the `aviso replay` subcommand.
pub(crate) async fn run(
    resolved: &Resolved,
    listener_files: &[PathBuf],
    listener_name: Option<&str>,
    inline: Option<ListenerSpec>,
    from: &str,
) -> Result<()> {
    let cursor: ResumeStart = from_value::parse(from)?;
    let spec = resolve_listener(resolved, listener_files, listener_name, inline)?;
    let listener_label = spec.name.clone().unwrap_or_else(|| spec.event.clone());

    print_replay_banner(&spec, from);

    let req = listener::build_replay_request(&spec, cursor);
    let triggers = collect_triggers(&spec);
    let req = req.with_triggers(triggers);

    // Replay is stateless by design: passing `None` here means the
    // supervisor never reads or writes the state file for this run.
    // Wiring `Some(store)` would let replay collide with `aviso listen`
    // on the same `ResumeKey` (derived from server URL + event type +
    // filter), and the monotonic-merge rule would let replay advance
    // listen's cursor past events listen has not processed, breaking
    // listen's at-least-once delivery. The trade-off is that an
    // interrupted replay is not auto-resumable; the operator passes
    // `--from <VALUE>` to seed the cursor on the next invocation.
    // See `docs/src/cli/replay.md` "Replay does not write the state
    // file" for the full reasoning.
    let client = client_builder::build(resolved, None, false)?;
    let mut cancel_rx = cancel::install();
    let mut stream = client.watch(req)?;
    let mut count: u64 = 0;
    let exit_result = loop {
        tokio::select! {
            biased;
            _ = cancel_rx.changed() => {
                let _ = output::write_stderr_line(&format!(
                    "Replay cancelled after {count} notification{plural}.",
                    plural = if count == 1 { "" } else { "s" },
                ));
                tracing::debug!(
                    event.name = "cli.replay.cancelled",
                    listener_name = %listener_label,
                    delivered = count,
                    "replay cancelled by signal; exiting cleanly"
                );
                break Ok(());
            }
            item = stream.recv() => {
                match item {
                    Some(Ok(notification)) => {
                        count = count.saturating_add(1);
                        tracing::debug!(
                            event.name = "cli.replay.notification",
                            listener_name = %listener_label,
                            event_type = %notification.event_type,
                            sequence = notification.sequence,
                            "received replay notification"
                        );
                    }
                    Some(Err(e)) => {
                        let hint = hint_for_replay_error(&e);
                        let mut err = anyhow::Error::from(e).context("draining replay stream");
                        if let Some(h) = hint {
                            err = err.context(format!("suggestion: {h}"));
                        }
                        break Err(err);
                    }
                    None => {
                        let _ = output::write_stderr_line(&format!(
                            "Replay complete. {count} notification{plural} delivered.",
                            plural = if count == 1 { "" } else { "s" },
                        ));
                        break Ok(());
                    }
                }
            }
        }
    };
    stream.close().await;
    exit_result
}

/// Emits a one-line summary of what is about to be replayed BEFORE the
/// supervisor opens the wire. Operators see this at default verbosity
/// (no `-v`); it replaces what would otherwise be silent output until
/// the first notification arrived, which on an empty filter could look
/// like the command had hung.
fn print_replay_banner(spec: &ListenerSpec, from: &str) {
    let name = spec.name.as_deref().unwrap_or(&spec.event);
    let filters = if spec.identifiers.is_empty() {
        String::new()
    } else {
        let mut parts: Vec<String> = spec
            .identifiers
            .iter()
            .map(|(k, v)| format!("{k}={}", render_identifier_value(v)))
            .collect();
        parts.sort();
        format!(" ({})", parts.join(", "))
    };
    let _ = output::write_stderr_line(&format!(
        "Replaying {name} [{}]{filters} from {}. Press Ctrl+C to stop.",
        spec.event,
        render_from_value(from),
    ));
}

/// Disambiguates the raw `--from` string for the operator-visible
/// banner. Plain digits always render as `sequence #N` (the parser
/// rule that pure-digit input is always a sequence id, never a
/// YYYYMMDD compact date). Any other input renders verbatim because
/// it is a date string that already speaks for itself.
fn render_from_value(raw: &str) -> String {
    if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        format!("sequence #{raw}")
    } else {
        raw.to_string()
    }
}

/// Maps a `ClientError` from the replay supervisor into a one-line
/// hint pointing at the operator's likely root cause. Mirrors the
/// notify and listen hint dispatchers so the same error class
/// produces the same hint regardless of which subcommand the
/// operator ran (UX consistency contract).
fn hint_for_replay_error(err: &aviso::ClientError) -> Option<String> {
    let aviso::ClientError::Http { status, body, .. } = err else {
        return None;
    };
    if body.contains("UNKNOWN_EVENT_TYPE") || body.contains("unknown event type") {
        return Some(
            "the event_type is not configured on the server. The response above includes a `configured_event_types` array; run `aviso schema list` for the same list. Check the `event:` value in your listener YAML or the `--event` argument for a typo."
                .to_string(),
        );
    }
    if body.contains("missing for watch operation") || body.contains("missing for replay operation")
    {
        return Some(
            "schema fields with `required: true` must be supplied in your replay identifiers (repeated `--identifier` or `--identifiers '{...}'` for ad-hoc replay, or the listener YAML's `identifiers:` block for YAML-driven replay); only `required: false` fields can be omitted (which makes them wildcards at replay time). Run `aviso schema get <TYPE>` to see which identifiers are `required: true`."
                .to_string(),
        );
    }
    if let Some(hint) = crate::commands::notify::polygon_violation_hint(body, "replay") {
        return Some(hint);
    }
    if let Some(hint) = crate::commands::notify::constraint_violation_hint(body, "replay") {
        return Some(hint);
    }
    match *status {
        401 => Some(
            "credentials are missing, invalid, or expired. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars; verify auth wired up via `aviso config dump --redact`."
                .to_string(),
        ),
        403 => Some(
            "credentials were accepted but may not have replay permission for this event_type. Contact the server admin; verify the event_type with `aviso schema list`."
                .to_string(),
        ),
        _ => None,
    }
}

/// Renders an identifier value for the banner WITHOUT the JSON quoting
/// that `serde_json::Value::Display` produces for string values. Mirrors
/// the same helper in `commands::listen` so both subcommands emit the
/// same operator-visible filter shape (`class=od`, not `class="od"`).
fn render_identifier_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Builds the trigger list for a replay session: a YAML-configured
/// listener uses its declared triggers as-is; the ad-hoc form (no
/// listener YAML at all) defaults to a single echo trigger so the
/// operator sees the replayed events on stdout. Without this default
/// the ad-hoc replay would silently receive and discard events,
/// which renders the subcommand useless for the inspection workflow
/// it exists to serve.
fn collect_triggers(spec: &ListenerSpec) -> Vec<Trigger> {
    if spec.triggers.is_empty() {
        let default = match spec.name.as_deref() {
            Some(name) => Trigger::echo().label(name),
            None => Trigger::echo(),
        };
        return vec![default];
    }
    listener::triggers_for_listener(spec)
}

fn resolve_listener(
    resolved: &Resolved,
    listener_files: &[PathBuf],
    selector: Option<&str>,
    inline: Option<ListenerSpec>,
) -> Result<ListenerSpec> {
    if let Some(spec) = inline {
        return Ok(spec);
    }

    let candidates = if listener_files.is_empty() {
        resolved.listeners.clone()
    } else {
        listener_file::load_concatenated(listener_files)?
    };

    if let Some(name) = selector {
        for spec in candidates {
            if spec.name.as_deref() == Some(name) {
                return Ok(spec);
            }
        }
        return Err(usage_error(format!(
            "no listener with name `{name}` found in the resolved listener set"
        )));
    }

    match candidates.len() {
        0 => Err(usage_error(
            "no listeners to replay. Pass --listener <NAME> with a listener YAML, or use --event with --identifiers or repeated --identifier for an ad-hoc replay.",
        )),
        1 => candidates
            .into_iter()
            .next()
            .ok_or_else(|| usage_error("internal: candidates len() == 1 but next() returned None")),
        n => {
            let names: Vec<String> = candidates
                .into_iter()
                .map(|s| s.name.unwrap_or(s.event))
                .collect();
            Err(usage_error(format!(
                "{n} listeners resolved; pass --listener <NAME> to pick one. Available names: {}",
                names.join(", ")
            )))
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic ListenerSpec construction is the expected diagnostic"
)]
mod tests {
    use super::*;
    use aviso::watch::TriggerConfig;
    use std::collections::BTreeMap;

    fn spec(triggers: Vec<TriggerConfig>) -> ListenerSpec {
        ListenerSpec {
            name: Some("t".into()),
            event: "mars".into(),
            identifiers: BTreeMap::new(),
            from_id: None,
            from_date: None,
            triggers,
        }
    }

    #[test]
    fn collect_triggers_defaults_to_one_echo_for_empty_spec() {
        let triggers = collect_triggers(&spec(Vec::new()));
        assert_eq!(
            triggers.len(),
            1,
            "an ad-hoc / triggers-empty replay MUST default to exactly one echo trigger so the operator sees the replayed events on stdout; without this the subcommand silently discards events and looks broken"
        );
    }

    #[test]
    fn collect_triggers_preserves_user_supplied_triggers_when_present() {
        let json_triggers = vec![
            serde_json::from_str::<TriggerConfig>(r#"{"type":"echo"}"#)
                .expect("echo JSON parses (TriggerConfig is format-agnostic)"),
            serde_json::from_str::<TriggerConfig>(r#"{"type":"log","path":"/tmp/x.log"}"#)
                .expect("log JSON parses"),
        ];
        let triggers = collect_triggers(&spec(json_triggers));
        assert_eq!(
            triggers.len(),
            2,
            "when the listener spec declares triggers, replay must use exactly those (no echo default appended): {triggers:?}",
        );
    }

    fn http_err(status: u16, body: &str) -> aviso::ClientError {
        aviso::ClientError::Http {
            status,
            body: body.to_string(),
            request_id: Some("req-test".into()),
        }
    }

    #[test]
    fn hint_for_replay_unknown_event_type_fires_with_typo_pointer() {
        let body = r#"{"code":"UNKNOWN_EVENT_TYPE","configured_event_types":["mars"],"message":"unknown event type 'xx'"}"#;
        let hint = hint_for_replay_error(&http_err(400, body)).expect(
            "UNKNOWN_EVENT_TYPE MUST yield a hint in replay (consistent with notify and listen)",
        );
        assert!(hint.contains("aviso schema list"), "{hint}");
        assert!(
            hint.contains("listener YAML") || hint.contains("--event"),
            "the replay-specific hint must mention BOTH listener YAML (file mode) AND --event (ad-hoc mode): {hint}"
        );
    }

    #[test]
    fn hint_for_replay_missing_required_field_fires_with_required_true_pointer() {
        let body = r#"{"details":"Required field 'polygon' missing for watch operation"}"#;
        let hint = hint_for_replay_error(&http_err(400, body))
            .expect("missing required field MUST yield a hint");
        assert!(hint.contains("required: true"), "{hint}");
        assert!(hint.contains("aviso schema get"), "{hint}");
        assert!(
            hint.contains("--identifiers JSON") || hint.contains("ad-hoc replay"),
            "the replay variant must also call out the ad-hoc form's --identifiers JSON option: {hint}"
        );
    }

    #[test]
    fn hint_for_replay_constraint_violation_delegates_to_shared_helper() {
        let body = r#"{"details":"Field 'class' exceeds maximum length of 2 characters, got: 3"}"#;
        let hint = hint_for_replay_error(&http_err(400, body))
            .expect("constraint violation MUST yield a hint in replay (consistent with notify and listen via shared helper)");
        assert!(hint.contains("aviso schema get"), "{hint}");
    }

    #[test]
    fn hint_for_replay_401_credentials() {
        let hint = hint_for_replay_error(&http_err(401, "{}")).expect("401 MUST yield a hint");
        assert!(hint.contains("credentials"), "{hint}");
        assert!(hint.contains("config dump"), "{hint}");
    }

    #[test]
    fn hint_for_replay_403_specifically_names_replay_permission() {
        let hint = hint_for_replay_error(&http_err(403, "{}")).expect("403 MUST yield a hint");
        assert!(
            hint.contains("replay permission"),
            "403 hint MUST specifically name `replay permission` (NOT `watch` or `notify` which would mislead): {hint}"
        );
    }

    #[test]
    fn hint_for_replay_unknown_status_returns_none() {
        assert!(hint_for_replay_error(&http_err(502, "<html>...</html>")).is_none());
    }

    #[test]
    fn hint_for_replay_non_http_client_error_returns_none() {
        let err = aviso::ClientError::Auth("test".into());
        assert!(hint_for_replay_error(&err).is_none());
    }

    #[test]
    fn render_from_value_pure_digits_render_as_sequence_id_to_disambiguate_from_yyyymmdd() {
        assert_eq!(
            render_from_value("42"),
            "sequence #42",
            "pure-digit input MUST render as `sequence #N` so an operator who typed `20260522` expecting a date sees `sequence #20260522` in the banner and immediately notices the misinterpretation (the parser rule that pure-digits always win is documented in `from_value::parse`): got {got:?}",
            got = render_from_value("42"),
        );
        assert_eq!(
            render_from_value("20260522"),
            "sequence #20260522",
            "the YYYYMMDD-trap case: the operator probably meant a date but the parser interprets it as a sequence id; the banner must spell this out so the operator catches the misinterpretation",
        );
    }

    #[test]
    fn render_from_value_date_forms_render_verbatim() {
        assert_eq!(render_from_value("2026-05-22"), "2026-05-22");
        assert_eq!(
            render_from_value("2026-05-22T12:34:56Z"),
            "2026-05-22T12:34:56Z"
        );
    }

    #[test]
    fn render_identifier_value_strips_json_quotes_from_string_values() {
        assert_eq!(
            render_identifier_value(&serde_json::Value::String("od".into())),
            "od",
            "operator-visible filters must render `class=od` (not `class=\"od\"`)"
        );
    }

    #[test]
    fn render_identifier_value_falls_through_to_json_display_for_non_strings() {
        assert_eq!(
            render_identifier_value(&serde_json::json!(42)),
            "42",
            "numeric identifier values keep their JSON Display rendering"
        );
        assert_eq!(
            render_identifier_value(&serde_json::json!(true)),
            "true",
            "boolean identifier values keep their JSON Display rendering"
        );
    }
}
