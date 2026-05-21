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
use tokio::sync::watch;
use tokio::task::{Id, JoinError, JoinSet};

use crate::cancel;
use crate::client_builder;
use crate::config::{ListenerSpec, Resolved};
use crate::exit::usage_error;
use crate::listener;
use crate::listener_file;

/// Runs the `aviso listen` subcommand.
pub(crate) async fn run(
    resolved: &Resolved,
    listener_files: &[PathBuf],
    _no_state_store: bool,
) -> Result<()> {
    let listeners = resolve_listeners(resolved, listener_files)?;
    if listeners.is_empty() {
        return Err(no_listeners_error(resolved, listener_files));
    }

    let client = Arc::new(client_builder::build(resolved)?);
    let cancel_rx = cancel::install();
    drive(client, listeners, cancel_rx).await
}

fn resolve_listeners(resolved: &Resolved, listener_files: &[PathBuf]) -> Result<Vec<ListenerSpec>> {
    if listener_files.is_empty() {
        Ok(resolved.listeners.clone())
    } else {
        listener_file::load_concatenated(listener_files)
    }
}

fn no_listeners_error(resolved: &Resolved, listener_files: &[PathBuf]) -> anyhow::Error {
    let positional = if listener_files.is_empty() {
        "(none given)".to_string()
    } else {
        listener_files
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    usage_error(format!(
        "no listeners to run.\nchecked positional arguments: {positional}\nchecked config file: {} (no `listeners:` section)\nfix: pass listener YAML files as positional arguments (e.g., `aviso listen my_listeners.yaml`), or add a `listeners:` block to the config file. See `aviso listen --help` for details.",
        resolved.config_path.display()
    ))
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
                tracing::debug!(
                    event.name = "cli.listener.exit_clean",
                    listener_name = %name,
                    "listener exited cleanly"
                );
            }
            Ok(Err(client_err)) => {
                tracing::warn!(
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
                tracing::error!(
                    event.name = "cli.listener.panic",
                    listener_name = %name,
                    %payload,
                    "listener panicked; other listeners continue"
                );
                any_failed = true;
            }
            Err(join_err) => {
                tracing::error!(
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
        Err(anyhow::anyhow!(
            "at least one listener errored or panicked; see WARN/ERROR events above"
        ))
    } else {
        Ok(())
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
