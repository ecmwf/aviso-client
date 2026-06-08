// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Command trigger dispatch.
//!
//! Spawns `/bin/sh -c <rendered>` per notification with the notification's
//! fields exposed as `AVISO_*` environment variables, captures the last
//! 4 KiB of stdout and stderr into ring buffers, and reports a typed
//! [`crate::watch::TriggerError::Command`] when the child exits non-zero.
//!
//! # Stdout suppression
//!
//! Command stdout may contain user payloads or secrets; per the
//! AGENTS.md no-payload-logging rule, only the captured byte count
//! reaches DEBUG-level tracing and the content is dropped immediately.
//! Stderr tail goes into the public `TriggerError::Command` variant on
//! failure because the operator chose to surface a failing command.
//!
//! # Shell descendant cleanup limitation
//!
//! `child.kill()` sends `SIGKILL` to the `/bin/sh -c ...` child only.
//! Pipelines, backgrounded jobs, and grandchildren that survive the
//! shell are NOT killed and may hold the stdout/stderr pipes open
//! beyond the dispatcher's expected lifetime. The 5-second post-exit
//! drain cap bounds the dispatcher's exposure even in this case;
//! operators who need full process-tree cleanup should use
//! `exec ./binary` to replace the shell in place.
//!
//! # POSIX-only
//!
//! Unix only. The whole module is gated behind `#[cfg(unix)]` at
//! the parent `trigger/mod.rs` declaration site; native Windows
//! support for the command trigger is not planned.

// The whole `command` module is gated behind `#[cfg(unix)]` at the
// parent `mod.rs` declaration site, so the compiler never visits
// this file on non-Unix targets. The previous `compile_error!` in
// this position was a belt-and-braces guard but is redundant under
// the parent gate and would prevent a non-Unix build from compiling
// even when the user only needs echo/log.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;

use crate::Notification;

use super::TriggerError;
use super::template::{CompiledTemplate, TemplateError, compile, template_error_to_trigger_error};

#[cfg(test)]
mod tests;

/// Bytes of stdout / stderr to retain per attempt.
const RING_CAP: usize = 4096;

/// Max time to wait for drain tasks after the child has exited; bounds
/// dispatcher exposure to backgrounded descendants holding the pipes
/// open after the shell exits.
const POST_EXIT_DRAIN_CAP: Duration = Duration::from_secs(5);

/// Configuration for the command trigger, held inside
/// [`super::TriggerKind::Command`].
///
/// The template-compile result is stored as-is: a parse failure does
/// not prevent constructing the `Trigger` (the
/// [`crate::watch::Trigger::command`] constructor is infallible to
/// match the existing `Trigger::echo`/`Trigger::log` pattern); instead
/// the error surfaces at first dispatch as
/// [`crate::watch::TriggerError::Template`].
#[derive(Clone)]
pub(super) struct CommandConfig {
    pub(super) command_template: Result<CompiledTemplate, TemplateError>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) working_dir: Option<PathBuf>,
}

/// Manual Debug impl that REDACTS the raw command template and the
/// env-var VALUES, because either can carry secrets (bearer tokens,
/// connection URIs). The redacted output names only structural facts:
/// whether the template compiled, whether a `working_dir` is set, and
/// the count of env vars the user supplied. Operators who need the
/// raw template for diagnosis use DEBUG-level tracing, which sees the
/// template through the `client.trigger.template.render_failed`
/// event field and never goes through Debug.
impl std::fmt::Debug for CommandConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let CommandConfig {
            command_template,
            env,
            working_dir,
        } = self;
        let template_state: &dyn std::fmt::Display = match command_template {
            Ok(_) => &"<compiled-template-redacted>",
            Err(_) => &"<bad-template-redacted>",
        };
        f.debug_struct("CommandConfig")
            .field("command_template", &format_args!("{template_state}"))
            .field("env_count", &env.len())
            .field("working_dir", working_dir)
            .finish()
    }
}

/// Dispatch a single command-trigger attempt.
///
/// On success returns `Ok(())`. On any failure returns
/// [`TriggerError`] carrying a typed reason: `Template { .. }` when
/// the command-string template could not be rendered, `Io(...)` when
/// the spawn or wait syscalls failed, `Command { exit_code,
/// stderr_tail }` when the child exited non-zero, and `Timeout(t)`
/// when the optional per-trigger timeout fired before the child
/// exited (the dispatcher kills and reaps the child before
/// returning).
#[allow(
    clippy::too_many_lines,
    reason = "function is intrinsically sequential (template render -> working_dir check -> spawn -> drain task spawn -> child wait -> post-exit drain join -> classify exit); extracting subphases adds parameter-passing complexity without improving readability"
)]
pub(super) async fn dispatch_command(
    cfg: &CommandConfig,
    timeout: Option<Duration>,
    notification: &Notification,
) -> Result<(), TriggerError> {
    let template = cfg
        .command_template
        .as_ref()
        .map_err(|e| template_error_to_trigger_error(e.clone(), "command"))?;
    let rendered_command = template
        .render(notification)
        .map_err(|e| template_error_to_trigger_error(e, "command"))?;

    if let Some(path) = &cfg.working_dir {
        if !tokio::fs::try_exists(path)
            .await
            .map_err(TriggerError::Io)?
        {
            return Err(TriggerError::Io(std::io::Error::other(format!(
                "working_dir {} does not exist",
                path.display()
            ))));
        }
        let meta = tokio::fs::metadata(path).await.map_err(TriggerError::Io)?;
        if !meta.is_dir() {
            return Err(TriggerError::Io(std::io::Error::other(format!(
                "working_dir {} is not a directory",
                path.display()
            ))));
        }
    }

    let injected_env = env_injection(notification)?;
    let mut cmd = TokioCommand::new("/bin/sh");
    cmd.arg("-c")
        .arg(&rendered_command)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(injected_env)
        .envs(&cfg.env);
    if let Some(path) = &cfg.working_dir {
        cmd.current_dir(path);
    }
    let mut child = cmd.spawn().map_err(TriggerError::Io)?;

    let stdout = child.stdout.take().ok_or_else(|| {
        TriggerError::Io(std::io::Error::other(
            "stdout pipe missing immediately after spawn; bug in tokio::process",
        ))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        TriggerError::Io(std::io::Error::other(
            "stderr pipe missing immediately after spawn; bug in tokio::process",
        ))
    })?;
    let stdout_task = tokio::spawn(drain_to_ring(stdout, RING_CAP, "stdout"));
    let stderr_task = tokio::spawn(drain_to_ring(stderr, RING_CAP, "stderr"));
    let stdout_abort = stdout_task.abort_handle();
    let stderr_abort = stderr_task.abort_handle();

    let exit_status = match timeout {
        Some(t) => {
            let sleep = tokio::time::sleep(t);
            tokio::pin!(sleep);
            tokio::select! {
                status = child.wait() => status.map_err(TriggerError::Io)?,
                () = &mut sleep => {
                    if let Err(e) = child.start_kill() {
                        tracing::warn!(
                            event.name = "client.trigger.command.kill_failed",
                            error = %e,
                            "failed to send SIGKILL to command child; may be already dead"
                        );
                    }
                    if let Err(e) = child.wait().await {
                        tracing::warn!(
                            event.name = "client.trigger.command.reap_failed",
                            error = %e,
                            "failed to reap killed command child"
                        );
                    }
                    stdout_abort.abort();
                    stderr_abort.abort();
                    return Err(TriggerError::Timeout(t));
                }
            }
        }
        None => child.wait().await.map_err(TriggerError::Io)?,
    };

    let drained = tokio::time::timeout(POST_EXIT_DRAIN_CAP, async {
        let s_out = stdout_task.await;
        let s_err = stderr_task.await;
        (s_out, s_err)
    })
    .await;

    let (stdout_tail, stderr_tail) = match drained {
        Ok((s_out, s_err)) => {
            let stdout = s_out.unwrap_or_else(|e| {
                tracing::warn!(
                    event.name = "client.trigger.command.stdout_drain_join_failed",
                    error = %e,
                    "stdout drain task join failed; captured output unavailable"
                );
                Vec::new()
            });
            let stderr = s_err.unwrap_or_else(|e| {
                tracing::warn!(
                    event.name = "client.trigger.command.stderr_drain_join_failed",
                    error = %e,
                    "stderr drain task join failed; captured output unavailable"
                );
                Vec::new()
            });
            (stdout, stderr)
        }
        Err(_elapsed) => {
            tracing::warn!(
                event.name = "client.trigger.command.post_exit_drain_capped",
                cap_seconds = POST_EXIT_DRAIN_CAP.as_secs(),
                "post-exit drain capped; backgrounded descendant likely holding pipes open"
            );
            stdout_abort.abort();
            stderr_abort.abort();
            (Vec::new(), Vec::new())
        }
    };

    tracing::debug!(
        event.name = "client.trigger.command.stdout_bytes",
        captured_bytes = stdout_tail.len(),
        "command stdout captured (content suppressed per no-payload-logging rule)"
    );
    drop(stdout_tail);

    if exit_status.success() {
        Ok(())
    } else {
        let stderr_tail_str = String::from_utf8_lossy(&stderr_tail).into_owned();
        Err(TriggerError::Command {
            exit_code: exit_status.code().unwrap_or(-1),
            stderr_tail: stderr_tail_str,
        })
    }
}

/// Build the dispatcher-injected env vars for the child.
///
/// Returns a Result so that a `serde_json::to_string` failure on the
/// notification serialisation surfaces as `TriggerError::Encode`
/// instead of silently dropping the env var. Order of returned tuples
/// matters: the dispatcher passes them to `Command::envs` BEFORE the
/// user-supplied `cfg.env`, so user keys override dispatcher-injected
/// keys when both are present.
fn env_injection(n: &Notification) -> Result<Vec<(String, String)>, TriggerError> {
    let mut out: Vec<(String, String)> = Vec::new();
    out.push(("AVISO_EVENT_TYPE".to_string(), n.event_type.clone()));
    out.push(("AVISO_SEQUENCE".to_string(), n.sequence.to_string()));
    for (k, v) in &n.identifier {
        out.push((format!("AVISO_IDENTIFIER_{}", normalize_key(k)), v.clone()));
    }
    let payload_json = serde_json::to_string(&n.payload).map_err(TriggerError::Encode)?;
    out.push(("AVISO_PAYLOAD_JSON".to_string(), payload_json));
    let notification_json = serde_json::to_string(n).map_err(TriggerError::Encode)?;
    out.push(("AVISO_NOTIFICATION_JSON".to_string(), notification_json));
    Ok(out)
}

/// Uppercase the key and replace non-alphanumerics with `_`, so
/// identifiers like `country-code` round-trip into
/// `AVISO_IDENTIFIER_COUNTRY_CODE` (env-var-safe).
pub(super) fn normalize_key(k: &str) -> String {
    k.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Drain `reader` into a bounded ring buffer of capacity `cap`,
/// retaining only the LAST `cap` bytes.
///
/// Read errors are logged at DEBUG with the supplied `stream_label`
/// and the buffer accumulated so far is returned; the captured tail
/// is best-effort diagnostic, and the authoritative success / fail
/// signal is the child's exit status.
pub(super) async fn drain_to_ring<R>(
    mut reader: R,
    cap: usize,
    stream_label: &'static str,
) -> Vec<u8>
where
    R: AsyncRead + Unpin,
{
    let mut ring: Vec<u8> = Vec::with_capacity(cap);
    let mut buf = vec![0u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let slice = &buf[..n];
                if ring.len() + n > cap {
                    if n >= cap {
                        ring.clear();
                        ring.extend_from_slice(&slice[n - cap..]);
                        continue;
                    }
                    let overflow = ring.len() + n - cap;
                    ring.drain(..overflow);
                }
                ring.extend_from_slice(slice);
            }
            Err(e) => {
                tracing::debug!(
                    event.name = "client.trigger.command.pipe_read_error",
                    stream = stream_label,
                    error = %e,
                    "pipe read error; degrading to partial tail"
                );
                break;
            }
        }
    }
    ring
}

/// Build a [`CommandConfig`] from a raw command string. The compile
/// result is stored in the config; render-time failures surface at
/// first dispatch.
pub(super) fn build_command_config(cmd: impl Into<String>) -> CommandConfig {
    let cmd_str = cmd.into();
    let compiled = compile(&cmd_str);
    CommandConfig {
        command_template: compiled,
        env: BTreeMap::new(),
        working_dir: None,
    }
}
