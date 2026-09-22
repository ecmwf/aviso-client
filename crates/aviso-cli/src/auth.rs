// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Auth-provider selection for the `aviso` binary.
//!
//! Four sources in highest-priority-first order per Q8 + amendment A2:
//!
//! 1. **Flag tier**: `--token <T>` -> `Bearer::new(T)`; else
//!    `--username <U>` + `--password <P>` -> `Basic::new(U, P)`;
//!    else no flag tier. Clap's `conflicts_with` rejects mixing
//!    `--token` with `--username`/`--password` at parse time.
//! 2. **Environment**: `AVISO_TOKEN`, or `AVISO_USERNAME` with
//!    `AVISO_PASSWORD`.
//! 3. **Config file**: the `auth:` block of the file the binary
//!    resolved, so `--config` is honoured.
//! 4. **Credentials file**: `credentials.yaml` in the aviso config
//!    directory, or the path in `AVISO_CREDENTIALS_FILE`. It is
//!    written by tools rather than by hand, so it ranks last and
//!    never overrides a credential the operator typed. It is the
//!    only source that re-reads on a 401, so a rotated token
//!    reaches a running listener.
//!
//! The last three are `aviso::auth::discover_with`, which is what
//! the library and the Python package use, so every surface agrees
//! on the order. The search stops at the first source that has a
//! credential: a later one is not read at all, and cannot fail a
//! command that was never going to use it.
//!
//! When no source has a credential the client runs anonymous; the
//! schema and health endpoints work this way against any
//! aviso-server.

use std::sync::Arc;

use anyhow::{Context, Result};
use aviso::ClientError;
use std::path::Path;

use aviso::auth::{AuthProvider, Basic, Bearer};

/// Builds an auth provider from the flag tier.
///
/// Returns `Ok(None)` if neither `--token` nor
/// `--username`/`--password` is supplied. Mutual exclusion between
/// `--token` and the basic-auth flags is enforced by clap at parse
/// time via `conflicts_with`, so this function trusts the input is
/// already mutually consistent.
///
/// # Errors
///
/// Propagates `Bearer::new` / `Basic::new` failures (e.g. empty
/// token) as `anyhow::Error`.
pub(crate) fn provider_from_flags(
    token: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Option<Arc<dyn AuthProvider>>> {
    if let Some(t) = token {
        let bearer =
            Bearer::new(t.to_string()).context("build Bearer auth provider from --token flag")?;
        return Ok(Some(Arc::new(bearer)));
    }
    if let (Some(u), Some(p)) = (username, password) {
        let basic = Basic::new(u.to_string(), p.to_string())
            .context("build Basic auth provider from --username/--password flags")?;
        return Ok(Some(Arc::new(basic)));
    }
    Ok(None)
}

/// Selects the provider and names the source it came from.
///
/// The flag tier is checked first because it is the most immediate
/// expression of intent. Everything below it is the shared search
/// in `aviso::auth::discover_with`, pointed at the config file this
/// invocation resolved.
///
/// A discovered credential is refused for a plaintext address that
/// is not loopback, because the caller did not choose to send it
/// there. The flag tier is exempt: writing `--token` on the command
/// line is choosing.
///
/// # Errors
///
/// Propagates a source that exists but cannot be used: a half-set
/// environment, or a file that cannot be read or parsed. A source
/// that is simply absent is skipped.
/// A selected provider and the name of the source it came from.
pub(crate) type SelectedProvider = (Option<Arc<dyn AuthProvider>>, Option<&'static str>);

pub(crate) fn resolve_provider(
    flag_provider: Option<Arc<dyn AuthProvider>>,
    config_path: &Path,
    base_url: Option<&str>,
) -> Result<SelectedProvider> {
    if let Some(provider) = flag_provider {
        return Ok((Some(provider), Some("flag")));
    }
    let paths = aviso::auth::DiscoveryPaths {
        config_file: Some(config_path.to_path_buf()),
        credentials_file: aviso::auth::DiscoveryPaths::from_env().credentials_file,
    };
    let found = match base_url {
        Some(url) => aviso::auth::discover_for_url(url, &paths),
        None => aviso::auth::discover_with(&paths),
    }
    .map_err(|e| match e {
        ClientError::Auth(reason) => crate::exit::usage_error(reason),
        other => anyhow::Error::from(other),
    })?;
    Ok(match found {
        Some(found) => {
            let label = match found.source() {
                aviso::auth::CredentialSource::Environment => "environment",
                aviso::auth::CredentialSource::ConfigFile(_) => "config file",
                aviso::auth::CredentialSource::CredentialsFile(_) => "credentials file",
            };
            (Some(found.into_provider()), Some(label))
        }
        None => (None, None),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on provider construction is the expected diagnostic"
)]
mod tests {
    use super::*;

    #[test]
    fn flag_token_yields_bearer_provider() {
        let p = provider_from_flags(Some("the-token"), None, None).unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn flag_username_password_yield_basic_provider() {
        let p = provider_from_flags(None, Some("alice"), Some("hunter2")).unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn flag_username_only_yields_none() {
        let p = provider_from_flags(None, Some("alice"), None).unwrap();
        assert!(p.is_none());
    }

    #[test]
    fn flag_empty_token_errors() {
        let err = provider_from_flags(Some(""), None, None).unwrap_err();
        let s = err.to_string();
        assert!(
            s.contains("Bearer") || s.contains("--token"),
            "error should name the source: {s}"
        );
    }

    #[test]
    fn empty_flag_yields_none() {
        let p = provider_from_flags(None, None, None).unwrap();
        assert!(p.is_none());
    }
}
