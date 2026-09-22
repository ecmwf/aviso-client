// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Materialise an [`aviso::AvisoClient`] from a [`crate::config::Resolved`].
//!
//! Single entry point [`build`] consumed by every subcommand
//! handler that needs to talk to aviso-server. Each handler should
//! call this once at the top of its `run` and then use the
//! returned client for the subcommand's HTTP calls.
//!
//! The function pulls TLS knobs off the resolved config and feeds
//! them through the `AvisoClientBuilder::ca_bundle` /
//! `.danger_accept_invalid_certs` setters added earlier in this
//! branch. Each `--ca-bundle` path is read once, parsed via
//! `reqwest::Certificate::from_pem`, and threaded into the
//! builder; PEM read or parse failures surface as a usage error
//! naming the offending file (per Error UX rule 1: file-related
//! input mistakes exit 2).

use std::sync::Arc;

use anyhow::{Context, Result};
use aviso::state::{JsonFileStore, MemoryStore, StateStore};
use aviso::{AvisoClient, AvisoClientBuilder};

use crate::config::Resolved;
use crate::exit::usage_error;
use crate::paths;

/// Builds an `AvisoClient` from the resolved CLI configuration.
///
/// `base_url` is required; absence is a usage error per Q3.
/// Optional fields (`timeout`, `heartbeat_interval`, `auth_provider`,
/// `ca_bundle`, `danger_accept_invalid_certs`) flow through to the
/// matching builder setters only when set.
///
/// `state_store` controls the supervisor's resume behaviour:
/// `Some(store)` wires the JsonFileStore (or MemoryStore when
/// `--no-state-store` is set); `None` leaves the supervisor without
/// a state store (no persistent resume).
///
/// `flush_cursor_on_exit` opts the supervisor into the post-loop
/// `pending_commit` flush. `aviso listen` (the only long-running,
/// operator-interactive subcommand) sets this to `true` so the LAST
/// notification of every session is durably persisted on graceful
/// exit and the operator does not see the same notification again on
/// the next run. Every other subcommand (notify, schema, admin,
/// replay) passes `false`: they are one-shot, do not configure a
/// state store, and have no `pending_commit` to flush.
pub(crate) fn build(
    resolved: &Resolved,
    state_store: Option<Arc<dyn StateStore>>,
    flush_cursor_on_exit: bool,
) -> Result<AvisoClient> {
    let base_url = resolved
        .base_url
        .as_ref()
        .ok_or_else(|| {
            usage_error(
                "base_url is required; set base_url in the config file, --base-url on the command line, or the AVISO_BASE_URL environment variable",
            )
        })?
        .value
        .clone();

    let mut builder: AvisoClientBuilder = AvisoClient::builder().base_url(&base_url);

    if let Some(t) = resolved.timeout.as_ref() {
        builder = builder.timeout(t.value);
    }
    if let Some(h) = resolved.heartbeat_interval.as_ref() {
        builder = builder.heartbeat_interval(h.value);
    }
    if let Some(provider) = resolved.auth_provider.as_ref() {
        // A credential the operator did not name on the command line is
        // not sent to a plaintext address unless it is loopback. This is
        // checked here, not at config resolution, so `config dump` can
        // still report which source would have been used.
        if resolved.auth_source != Some("flag")
            && !aviso::auth::url_keeps_credentials_private(&base_url)
        {
            return Err(usage_error(format!(
                "refusing to send the credential from the {} to {}, which is not https and not a loopback address. Use an https address, or pass the credential with --token or --username/--password to say you mean it.",
                resolved.auth_source.unwrap_or("configured source"),
                aviso::auth::url_without_userinfo(&base_url),
            )));
        }
        builder = builder.auth(Arc::clone(provider));
    }
    for path in &resolved.tls_ca_bundle_paths.value {
        let bytes = std::fs::read(path).map_err(|e| {
            usage_error(format!(
                "could not read --ca-bundle PEM file `{}`: {e}",
                path.display()
            ))
        })?;
        let cert = reqwest::Certificate::from_pem(&bytes).map_err(|e| {
            usage_error(format!(
                "--ca-bundle PEM file `{}` did not parse as an X.509 certificate: {e}",
                path.display()
            ))
        })?;
        builder = builder.ca_bundle(cert);
    }
    if resolved.tls_danger_accept_invalid_certs.value {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(store) = state_store {
        builder = builder.state_store(store);
    }
    if flush_cursor_on_exit {
        builder = builder.flush_cursor_on_exit(true);
    }

    builder.build().context("build aviso client")
}

/// Resolves the per-invocation [`StateStore`] for the listen / replay
/// subcommands.
///
/// `--no-state-store` -> [`MemoryStore`].
/// Otherwise [`JsonFileStore::open`] against the resolved state path;
/// the parent directory is created via [`paths::ensure_secure_dir`]
/// on first call so an out-of-the-box `~/.config/aviso/` invocation
/// succeeds without asking the operator to `mkdir -p` ahead of time.
/// On Unix, the helper applies mode `0o700` to every newly-created
/// directory along the path so the state journal (which can carry
/// auth-bearing resume cursors) is not world-readable.
pub(crate) async fn build_state_store(
    resolved: &Resolved,
    no_state_store: bool,
) -> Result<Arc<dyn StateStore>> {
    if no_state_store {
        return Ok(Arc::new(MemoryStore::new()) as Arc<dyn StateStore>);
    }
    let path = &resolved.state_path.value;
    if let Some(parent) = path.parent() {
        paths::ensure_secure_dir(parent).with_context(|| {
            format!(
                "create parent directory for state file: {}",
                parent.display()
            )
        })?;
    }
    let store = JsonFileStore::open(path.clone())
        .await
        .with_context(|| format!("open JsonFileStore at {}", path.display()))?;
    Ok(Arc::new(store) as Arc<dyn StateStore>)
}
