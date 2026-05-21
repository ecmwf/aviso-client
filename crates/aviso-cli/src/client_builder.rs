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
pub(crate) fn build(
    resolved: &Resolved,
    state_store: Option<Arc<dyn StateStore>>,
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

    let mut builder: AvisoClientBuilder = AvisoClient::builder().base_url(base_url);

    if let Some(t) = resolved.timeout.as_ref() {
        builder = builder.timeout(t.value);
    }
    if let Some(h) = resolved.heartbeat_interval.as_ref() {
        builder = builder.heartbeat_interval(h.value);
    }
    if let Some(provider) = resolved.auth_provider.as_ref() {
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
