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
//! naming the offending file.

use std::sync::Arc;

use anyhow::{Context, Result};
use aviso::{AvisoClient, AvisoClientBuilder};

use crate::config::Resolved;
use crate::exit::usage_error;

/// Builds an `AvisoClient` from the resolved CLI configuration.
///
/// `base_url` is required; absence is a usage error per Q3.
/// Optional fields (`timeout`, `heartbeat_interval`, `auth_provider`,
/// `ca_bundle`, `danger_accept_invalid_certs`) flow through to the
/// matching builder setters only when set.
pub(crate) fn build(resolved: &Resolved) -> Result<AvisoClient> {
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
        let bytes = std::fs::read(path)
            .with_context(|| format!("read --ca-bundle PEM file: {}", path.display()))?;
        let cert = reqwest::Certificate::from_pem(&bytes)
            .with_context(|| format!("parse --ca-bundle PEM file: {}", path.display()))?;
        builder = builder.ca_bundle(cert);
    }
    if resolved.tls_danger_accept_invalid_certs.value {
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder.build().context("build aviso client")
}
