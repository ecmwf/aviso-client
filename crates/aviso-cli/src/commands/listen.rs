//! `aviso listen` subcommand.
//!
//! Per Amendments C, D, E, G:
//!
//! C. Variadic positional `[LISTENER_FILE...]` REPLACES the
//!    global config's `listeners:` block when supplied. With no
//!    positional and no global listeners, exit 2 with a helpful
//!    error naming both paths checked.
//! D. Every resolved listener spawns concurrently via
//!    `tokio::task::JoinSet`.
//! E. Stdout stays empty; triggers handle output.
//! G. One listener erroring or panicking does NOT cancel
//!    siblings. The CLI exits 1 iff `any_failed`, otherwise 0.
//!
//! Cancellation: every spawned listener receives a cloned
//! `watch::Receiver<bool>` from `cancel::install()`. First Ctrl+C
//! flips the watch; each task drops its stream and returns
//! `Ok(())`. Second Ctrl+C within 5s hard-exits 130 via the
//! signal-handler task in `cancel.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use aviso::watch::ResumeStart;
use tokio::sync::watch;
use tokio::task::{Id, JoinError, JoinSet};

use crate::cancel;
use crate::client_builder;
use crate::config::{ListenerSpec, Resolved};
use crate::exit::usage_error;
use crate::from_value;
use crate::listener;
use crate::listener_file;
use crate::output;
use crate::paths;

/// Runs the `aviso listen` subcommand.
pub(crate) async fn run(
    resolved: &Resolved,
    listener_files: &[PathBuf],
    no_state_store: bool,
    from: Option<&str>,
) -> Result<()> {
    let mut listeners = resolve_listeners(resolved, listener_files)?;
    if listeners.is_empty() {
        return Err(no_listeners_error(resolved, listener_files));
    }

    if let Some(value) = from {
        let cursor = from_value::parse(value)?;
        apply_cursor_override(&mut listeners, &cursor);
    }

    print_startup_banner(&listeners);

    let state_store = client_builder::build_state_store(resolved, no_state_store).await?;
    let client = Arc::new(client_builder::build(resolved, Some(state_store), true)?);
    let cancel_rx = cancel::install();
    drive(client, listeners, cancel_rx).await
}

/// Emits a user-facing one-line summary of what each listener is
/// subscribed to, BEFORE the supervisor starts connecting. Operators
/// see this at default verbosity (no `-v`); it replaces what would
/// otherwise be a series of structured `tracing` events the operator
/// would have to parse to figure out which listeners are active.
fn print_startup_banner(listeners: &[ListenerSpec]) {
    let mut summaries: Vec<String> = Vec::with_capacity(listeners.len());
    for spec in listeners {
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
        summaries.push(format!("{name} [{}]{filters}", spec.event));
    }
    let _ = output::write_stderr_line(&format!(
        "Listening for {}. Press Ctrl+C to stop.",
        summaries.join(", ")
    ));
}

/// Renders an identifier value for the startup banner WITHOUT the
/// JSON quoting that `serde_json::Value::Display` produces for
/// string values. The `BTreeMap<String, serde_json::Value>` storage
/// is for schema flexibility (numeric / bool values are possible);
/// for the operator-visible banner we want `class=od` not
/// `class="od"`. Non-string values fall through to the normal JSON
/// Display.
fn render_identifier_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn apply_cursor_override(listeners: &mut [ListenerSpec], cursor: &ResumeStart) {
    for spec in listeners.iter_mut() {
        spec.from_id = None;
        spec.from_date = None;
        match cursor {
            ResumeStart::AfterSequence(n) => spec.from_id = Some(*n),
            ResumeStart::Date(d) => spec.from_date = Some(d.clone()),
            _ => {}
        }
    }
}

fn resolve_listeners(resolved: &Resolved, listener_files: &[PathBuf]) -> Result<Vec<ListenerSpec>> {
    if listener_files.is_empty() {
        Ok(resolved.listeners.clone())
    } else {
        listener_file::load_concatenated(listener_files)
    }
}

fn no_listeners_error(resolved: &Resolved, listener_files: &[PathBuf]) -> anyhow::Error {
    let mut err = usage_error("no listeners to run");

    if listener_files.is_empty() {
        // No positional files supplied: resolution came from the
        // global config's `listeners:` block. The block may be
        // absent, present-but-empty (`listeners: []`), or
        // commented out; we cannot tell which from the resolved
        // value alone. Attribute the empty resolution to the
        // config path without claiming the section is absent.
        err = err.context(format!(
            "at: {} (the `listeners:` block resolved to 0 entries; the section may be absent, present-but-empty, or commented out)",
            resolved.config_path.value.display()
        ));
        err = err.context(
            "suggestion: pass listener YAML files as positional arguments (e.g., `aviso listen my_listeners.yaml`), or add a non-empty `listeners:` block to the config file. See `aviso listen --help` for details.",
        );
    } else {
        // Positional files were supplied: Amendment C semantics
        // mean those REPLACE the global config's `listeners:` for
        // this invocation, so the config file is intentionally not
        // consulted here. Attribute the empty resolution to the
        // positional files instead of mentioning the config path.
        // Each path is rendered absolute via paths::absolutize so
        // the resulting at: lines obey Error UX rule 3 regardless
        // of whether the operator typed a relative or absolute path
        // on the command line.
        for path in listener_files {
            let display_path = paths::absolutize(path).unwrap_or_else(|_| path.clone());
            err = err.context(format!(
                "at: {} (resolved to 0 entries)",
                display_path.display()
            ));
        }
        err = err.context(
            "suggestion: ensure each positional listener file contains a non-empty `listeners:` block, or omit the positional arguments to fall back to the global config's `listeners:` block. See `aviso listen --help` for details.",
        );
    }

    err
}

async fn drive(
    client: Arc<aviso::AvisoClient>,
    listeners: Vec<ListenerSpec>,
    cancel_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut join_set: JoinSet<Result<(), aviso::ClientError>> = JoinSet::new();
    let mut id_to_name: HashMap<Id, String> = HashMap::new();

    for spec in listeners {
        let req = listener::build_watch_request(&spec)?;
        let listener_name = spec.name.clone().unwrap_or_else(|| spec.event.clone());
        let event_type = spec.event.clone();
        let client_arc = Arc::clone(&client);
        let cancel_clone = cancel_rx.clone();
        let task_name = listener_name.clone();
        let abort = join_set.spawn(async move {
            listener::spawn_listener_drain(client_arc, req, cancel_clone, task_name, event_type)
                .await
        });
        id_to_name.insert(abort.id(), listener_name);
    }

    let mut any_failed = false;
    while let Some(item) = join_set.join_next_with_id().await {
        let (task_id, outcome): (Id, Result<Result<(), aviso::ClientError>, JoinError>) = match item
        {
            Ok((id, res)) => (id, Ok(res)),
            Err(join_err) => (join_err.id(), Err(join_err)),
        };
        let name = id_to_name
            .remove(&task_id)
            .unwrap_or_else(|| "<unknown>".to_string());
        match outcome {
            Ok(Ok(())) => {
                // Clean exit is already logged by `cli.listener.end_of_stream`
                // inside the per-listener task (`listener::spawn_listener_drain`).
                // A second DEBUG event from the supervisor join point would
                // be a duplicate of the same lifecycle transition.
            }
            Ok(Err(client_err)) => {
                let _ =
                    output::write_stderr_line(&format!("Error in listener {name}: {client_err}"));
                if let Some(hint) = hint_for_listener_error(&client_err) {
                    let _ = output::write_stderr_line(&format!("  Hint: {hint}"));
                }
                let _ = output::write_stderr_line("  Other listeners continue.");
                tracing::debug!(
                    event.name = "cli.listener.failed",
                    listener_name = %name,
                    error = %client_err,
                    "listener errored; other listeners continue"
                );
                any_failed = true;
            }
            Err(join_err) if join_err.is_panic() => {
                let panic_box = join_err.into_panic();
                let payload = format_panic_payload(panic_box.as_ref());
                let _ = output::write_stderr_line(&format!("Listener {name} panicked: {payload}"));
                let _ = output::write_stderr_line("  Other listeners continue.");
                tracing::debug!(
                    event.name = "cli.listener.panic",
                    listener_name = %name,
                    %payload,
                    "listener panicked; other listeners continue"
                );
                any_failed = true;
            }
            Err(join_err) => {
                let _ = output::write_stderr_line(&format!(
                    "Listener {name} task was cancelled unexpectedly: {join_err}"
                ));
                tracing::debug!(
                    event.name = "cli.listener.task_cancelled",
                    listener_name = %name,
                    error = %join_err,
                    "listener task was cancelled unexpectedly"
                );
                any_failed = true;
            }
        }
    }

    if any_failed {
        let _ = output::write_stderr_line(
            "Some listeners stopped with errors (see messages above). Exiting with non-zero status.",
        );
        Err(anyhow::anyhow!("one or more listeners errored or panicked"))
    } else {
        let _ = output::write_stderr_line("All listeners stopped.");
        Ok(())
    }
}

/// Inspects a [`aviso::ClientError`] for known operator-mistake
/// patterns specific to the watch / listener path and returns a
/// one-line hint when a match is found. Parallel to
/// [`crate::commands::notify::hint_for_client_error`] but with
/// messages tuned to listener-YAML mistakes: the schema's
/// `required: false` flag IS a wildcard at watch time, so the
/// "you forgot a field" case has different semantics than the
/// notify equivalent (only `required: true` fields are mandatory).
///
/// Patterns chosen are high-confidence (the server's own error
/// body unambiguously names the cause); when no pattern matches we
/// return `None` and let the raw `error` field on the
/// `cli.listener.failed` event speak for itself.
fn hint_for_listener_error(err: &aviso::ClientError) -> Option<String> {
    let aviso::ClientError::Http { status, body, .. } = err else {
        return None;
    };
    if body.contains("UNKNOWN_EVENT_TYPE") || body.contains("unknown event type") {
        return Some(
            "the event_type is not configured on the server. The response above includes a `configured_event_types` array listing every event_type the server accepts; run `aviso schema list` for the same list. Check for a typo in `event:` in your listener YAML."
                .to_string(),
        );
    }
    if body.contains("missing for watch operation") {
        return Some(
            "schema fields with `required: true` must appear in your listener YAML's `identifiers:` block; only `required: false` fields can be omitted (which makes them wildcards at watch time). Run `aviso schema get <TYPE>` to see which identifiers are `required: true`."
                .to_string(),
        );
    }
    if body.contains("must be a valid polygon") {
        let specific = if body.contains("odd number of values") {
            "coordinates must come in lat,lon pairs (even total count)"
        } else if body.contains("could not parse latitude")
            || body.contains("could not parse longitude")
        {
            "each coordinate must be a number; check for typos and confirm you're using comma (not semicolon) as the delimiter"
        } else if body.contains("at least 4 coordinate pairs") {
            "a polygon needs at least 4 coordinate pairs: 3 unique vertices plus a closing repeat of the first vertex"
        } else if body.contains("polygon coordinate string is empty") {
            "polygon value cannot be empty"
        } else if body.contains("outside the valid range") {
            "latitude must be in [-90, 90] and longitude in [-180, 180] (note the order: each pair is `lat,lon`, not `lon,lat`)"
        } else {
            "polygon must be a comma-separated list of lat,lon pairs"
        };
        return Some(format!(
            "{specific}. In listener YAML, set `polygon: \"46,8,46,9,47,9,47,8,46,8\"` (the value wrapped as a single string)."
        ));
    }
    match *status {
        401 => Some(
            "credentials are missing, invalid, or expired. Check --token / --username / --password or the AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD env vars; verify auth wired up via `aviso config dump --redact`."
                .to_string(),
        ),
        403 => Some(
            "credentials were accepted but may not have watch permission for this event_type. Contact the server admin; verify the event_type with `aviso schema list`."
                .to_string(),
        ),
        _ => None,
    }
}

fn format_panic_payload(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic ClientError fixtures is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn http_err(status: u16, body: &str) -> aviso::ClientError {
        aviso::ClientError::Http {
            status,
            body: body.to_string(),
            request_id: Some("req-test".into()),
        }
    }

    #[test]
    fn hint_for_unknown_event_type_points_at_event_field_in_yaml() {
        let body = r#"{"code":"UNKNOWN_EVENT_TYPE","configured_event_types":["dissemination","mars","test_polygon"],"message":"unknown event type 'marse'"}"#;
        let hint = hint_for_listener_error(&http_err(400, body))
            .expect("UNKNOWN_EVENT_TYPE must yield a hint");
        assert!(hint.contains("aviso schema list"), "{hint}");
        assert!(hint.contains("listener YAML"), "{hint}");
    }

    #[test]
    fn hint_for_missing_watch_field_calls_out_required_true_vs_false() {
        let body = r#"{"code":"INVALID_WATCH_REQUEST","details":"Required field 'polygon' missing for watch operation"}"#;
        let hint = hint_for_listener_error(&http_err(400, body))
            .expect("missing watch field must yield a hint");
        assert!(hint.contains("required: true"), "{hint}");
        assert!(
            hint.contains("required: false") && hint.contains("wildcard"),
            "must contrast against required: false (which IS a wildcard at watch time): {hint}"
        );
        assert!(hint.contains("aviso schema get"), "{hint}");
    }

    #[test]
    fn hint_for_polygon_format_in_listener_yaml() {
        let body = r#"{"details":"field 'polygon' must be a valid polygon: polygon coordinates must be in lat,lon pairs (got an odd number of values)"}"#;
        let hint = hint_for_listener_error(&http_err(400, body))
            .expect("polygon format must yield a hint");
        assert!(hint.contains("lat,lon pairs"), "{hint}");
        assert!(
            hint.contains("listener YAML") || hint.contains("polygon:"),
            "the listener variant of the hint must explicitly point at YAML syntax: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_unknown_sub_message_in_listener_falls_back_to_generic() {
        let body = r#"{"details":"field 'polygon' must be a valid polygon: brand new server validation we don't know about yet"}"#;
        let hint = hint_for_listener_error(&http_err(400, body)).expect(
            "listener polygon hint must always fire for any 'must be a valid polygon' body",
        );
        assert!(
            hint.contains("comma-separated list of lat,lon pairs"),
            "generic fallback must spell out the basic format: {hint}"
        );
    }

    #[test]
    fn hint_for_polygon_lat_lon_out_of_range_in_listener_yaml() {
        let body = r#"{"details":"field 'polygon' must be a valid polygon: latitude 91 is outside the valid range [-90, 90]"}"#;
        let hint = hint_for_listener_error(&http_err(400, body))
            .expect("listener polygon out-of-range MUST yield a hint");
        assert!(
            hint.contains("[-90, 90]") && hint.contains("[-180, 180]"),
            "the listener variant of the out-of-range hint must restate both ranges so the operator does not need to re-check the orthogonal axis manually: {hint}"
        );
        assert!(
            hint.contains("not `lon,lat`"),
            "the `lat,lon` order mnemonic must appear in the listener variant too: this is the most common operator mistake regardless of subcommand: {hint}"
        );
        assert!(
            hint.contains("listener YAML") || hint.contains("polygon:"),
            "the listener variant must explicitly point at YAML syntax: {hint}"
        );
    }

    #[test]
    fn hint_for_401_credentials_in_listener_path() {
        let hint = hint_for_listener_error(&http_err(401, "{}")).expect("401 must yield a hint");
        assert!(hint.contains("credentials"), "{hint}");
        assert!(hint.contains("config dump"), "{hint}");
    }

    #[test]
    fn hint_for_403_watch_permission_specific() {
        let hint = hint_for_listener_error(&http_err(403, "{}")).expect("403 must yield a hint");
        assert!(
            hint.contains("watch permission"),
            "must specifically name watch permission (not notify permission): {hint}"
        );
    }

    #[test]
    fn hint_for_unknown_status_returns_none() {
        assert!(hint_for_listener_error(&http_err(502, "<html>...</html>")).is_none());
    }

    #[test]
    fn hint_for_non_http_client_error_returns_none() {
        let err = aviso::ClientError::Auth("test".into());
        assert!(hint_for_listener_error(&err).is_none());
    }
}
