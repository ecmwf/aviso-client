//! Auth-provider chain construction for the `aviso` binary.
//!
//! Three tiers in highest-priority-first order per Q8 + amendment A2:
//!
//! 1. **Flag tier**: `--token <T>` -> `Bearer::new(T)`; else
//!    `--username <U>` + `--password <P>` -> `Basic::new(U, P)`;
//!    else no flag tier. Clap's `conflicts_with` rejects mixing
//!    `--token` with `--username`/`--password` at parse time.
//! 2. **Env tier**: `aviso::auth::Env::from_process_env()` (lib
//!    helper that reads `AVISO_TOKEN`, falling back to
//!    `AVISO_USERNAME` + `AVISO_PASSWORD`). The lib returns
//!    `Err(ClientError::Auth)` when no env credentials are set;
//!    this module treats that specific error as "no env tier
//!    available" (anonymous fallback) rather than propagating the
//!    error. Any OTHER env-tier error (e.g. non-UTF-8 env value
//!    surfacing as `ClientError::Config`) DOES propagate so the
//!    operator sees the misconfiguration.
//! 3. **File tier**: the parsed `[auth]` block from `config.yaml`,
//!    rendered through `Bearer::new` or `Basic::new`. Optional;
//!    a file without `auth:` contributes nothing.
//!
//! Assembled into `aviso::auth::Chain::new(vec![...])` (skipping
//! `None` tiers). The chain is passed to
//! `AvisoClientBuilder::auth(Arc::new(chain))`.
//!
//! When all three tiers are empty, the client runs anonymous; the
//! schema and health endpoints work this way against any
//! aviso-server.

use std::sync::Arc;

use anyhow::{Context, Result};
use aviso::ClientError;
use aviso::auth::{AuthProvider, Basic, Bearer, Chain, Env};

use crate::config::AuthConfig;

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

/// Reads credentials from the process environment.
///
/// Returns `Ok(None)` when neither `AVISO_TOKEN` nor
/// `AVISO_USERNAME`+`AVISO_PASSWORD` is set (the lib's
/// `from_process_env` returns `Err(ClientError::Auth)` in that
/// case; this wrapper translates that into "no env tier available").
/// Propagates all other errors verbatim: non-UTF-8 env values
/// surface as `ClientError::Config` from the lib and DO propagate.
pub(crate) fn provider_from_env() -> Result<Option<Arc<dyn AuthProvider>>> {
    match Env::from_process_env() {
        Ok(env) => Ok(Some(Arc::new(env))),
        Err(ClientError::Auth(_)) => Ok(None),
        Err(other) => {
            Err(anyhow::Error::from(other)).context("read auth credentials from environment")
        }
    }
}

/// Builds an auth provider from the file tier (a parsed `[auth]`
/// block).
///
/// Returns `Ok(None)` when the block is absent OR neither
/// `bearer_token` nor `basic` is set inside it.
///
/// # Errors
///
/// Propagates `Bearer::new` / `Basic::new` failures (e.g. empty
/// token in the config file) as `anyhow::Error`.
pub(crate) fn provider_from_file(
    cfg: Option<&AuthConfig>,
) -> Result<Option<Arc<dyn AuthProvider>>> {
    let Some(cfg) = cfg else {
        return Ok(None);
    };
    if let Some(token) = cfg.bearer_token.as_deref() {
        let bearer = Bearer::new(token.to_string())
            .context("build Bearer auth provider from config-file auth.bearer_token")?;
        return Ok(Some(Arc::new(bearer)));
    }
    if let Some(basic) = cfg.basic.as_ref() {
        let basic = Basic::new(basic.username.clone(), basic.password.clone())
            .context("build Basic auth provider from config-file auth.basic")?;
        return Ok(Some(Arc::new(basic)));
    }
    Ok(None)
}

/// Composes the final auth provider from the three tiers.
///
/// Returns `None` (anonymous) when all tiers are empty. With a
/// single tier the provider is returned directly (no `Chain` wrap
/// because that would add an indirection without behavioural
/// difference). With multiple tiers a `Chain` wraps them; the
/// `Chain` returns the first sub-provider's
/// `authorization_header()` that succeeds, in highest-priority
/// order.
pub(crate) fn build_chain(
    flag_provider: Option<Arc<dyn AuthProvider>>,
    env_provider: Option<Arc<dyn AuthProvider>>,
    file_provider: Option<Arc<dyn AuthProvider>>,
) -> Option<Arc<dyn AuthProvider>> {
    let providers: Vec<Arc<dyn AuthProvider>> = [flag_provider, env_provider, file_provider]
        .into_iter()
        .flatten()
        .collect();
    match providers.len() {
        0 => None,
        1 => providers.into_iter().next(),
        _ => Some(Arc::new(Chain::new(providers))),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on chain construction is the expected diagnostic"
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

    #[test]
    fn file_provider_none_when_block_absent() {
        let p = provider_from_file(None).unwrap();
        assert!(p.is_none());
    }

    #[test]
    fn file_provider_bearer_from_block() {
        let cfg = AuthConfig {
            bearer_token: Some("from-file".into()),
            basic: None,
        };
        let p = provider_from_file(Some(&cfg)).unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn chain_empty_returns_none() {
        let chain = build_chain(None, None, None);
        assert!(chain.is_none());
    }

    #[test]
    fn chain_single_tier_returns_that_tier_unwrapped() {
        let token = provider_from_flags(Some("flag-token"), None, None).unwrap();
        let chain = build_chain(token, None, None);
        assert!(chain.is_some());
    }

    #[test]
    fn chain_multiple_tiers_assembled_into_chain() {
        let flag = provider_from_flags(Some("flag-token"), None, None).unwrap();
        let file = provider_from_file(Some(&AuthConfig {
            bearer_token: Some("file-token".into()),
            basic: None,
        }))
        .unwrap();
        let chain = build_chain(flag, None, file);
        assert!(chain.is_some());
    }
}
