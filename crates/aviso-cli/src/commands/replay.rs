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
//! - `--event` + `--identifiers <JSON>` together override the
//!   listener spec entirely for an ad-hoc replay (both required or
//!   neither, enforced by clap).
//!
//! `--from <VALUE>` is mandatory and parsed via
//! [`crate::from_value`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use aviso::watch::ResumeStart;

use crate::cancel;
use crate::client_builder;
use crate::config::{ListenerSpec, Resolved};
use crate::exit::usage_error;
use crate::from_value;
use crate::listener;
use crate::listener_file;

/// Runs the `aviso replay` subcommand.
pub(crate) async fn run(
    resolved: &Resolved,
    listener_files: &[PathBuf],
    listener_name: Option<&str>,
    event: Option<&str>,
    identifiers: Option<&str>,
    from: &str,
) -> Result<()> {
    let cursor: ResumeStart = from_value::parse(from)?;
    let spec = resolve_listener(resolved, listener_files, listener_name, event, identifiers)?;
    let req = listener::build_replay_request(&spec, cursor);

    let client = client_builder::build(resolved, None)?;
    let mut cancel_rx = cancel::install();
    let mut stream = client.watch(req)?;
    loop {
        tokio::select! {
            biased;
            _ = cancel_rx.changed() => {
                tracing::info!(
                    event.name = "cli.replay.cancelled",
                    listener_name = %spec.name.as_deref().unwrap_or(&spec.event),
                    "replay cancelled by signal; exiting cleanly"
                );
                return Ok(());
            }
            item = stream.recv() => {
                match item {
                    Some(Ok(notification)) => {
                        tracing::debug!(
                            event.name = "cli.replay.notification",
                            listener_name = %spec.name.as_deref().unwrap_or(&spec.event),
                            event_type = %notification.event_type,
                            sequence = notification.sequence,
                            "received replay notification"
                        );
                    }
                    Some(Err(e)) => {
                        return Err(e).context("draining replay stream");
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}

fn resolve_listener(
    resolved: &Resolved,
    listener_files: &[PathBuf],
    selector: Option<&str>,
    event: Option<&str>,
    identifiers: Option<&str>,
) -> Result<ListenerSpec> {
    if let (Some(ev), Some(idents_json)) = (event, identifiers) {
        let identifiers: BTreeMap<String, serde_json::Value> = serde_json::from_str(idents_json)
            .map_err(|e| {
                usage_error(format!(
                    "parse --identifiers as JSON object: {e}; expected something like '{{\"class\":\"od\"}}'"
                ))
            })?;
        return Ok(ListenerSpec {
            name: Some("ad-hoc".into()),
            event: ev.to_string(),
            identifiers,
            from_id: None,
            from_date: None,
            triggers: Vec::new(),
        });
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
            "no listeners to replay. Pass --listener <NAME> with a listener YAML, or use --event with --identifiers for an ad-hoc replay.",
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
