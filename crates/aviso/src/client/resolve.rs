// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The settings a client will use, each with where it came from.
//!
//! A client built with no arguments takes its server address, timeouts and
//! credential from the environment and the aviso config file, the way the
//! `aviso` command does. When something does not connect, the first question
//! is which server and which credential the client ended up with. This module
//! answers it: [`resolve`] reads every source in order and returns a
//! [`ResolvedSettings`] naming, for each field, the value and its
//! [`Source`]. The client builder uses the same function, so what a dump
//! shows is what a client built from the same inputs would use.
//!
//! Precedence, highest first: a value the caller passed in code, the
//! environment (`AVISO_BASE_URL` for the address; `AVISO_TOKEN` or
//! `AVISO_USERNAME` with `AVISO_PASSWORD` for the credential), the config
//! file, the credentials file (credential only), and the built-in default.
//! [`EnvAddress`] says whether `AVISO_BASE_URL` takes part: it does for a
//! client built with no arguments, and does not for
//! [`AvisoClientBuilder::from_file`](crate::AvisoClientBuilder::from_file),
//! which reads the address from the file only.
//!
//! Nothing here holds a secret. The credential is described by its kind and
//! source, and the address is stored with any `user:password@` removed.

use std::path::PathBuf;
use std::time::Duration;

use super::settings::ClientSettings;
use crate::auth::{CredentialSource, Discovered, DiscoveryPaths};

/// Where a resolved value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    /// Passed by the caller in code.
    Code,
    /// Read from the named environment variable.
    Environment(&'static str),
    /// Read from the config file at this path.
    ConfigFile(PathBuf),
    /// Read from the credentials file at this path.
    CredentialsFile(PathBuf),
    /// Nothing supplied it; this is the built-in default.
    Default,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Code => write!(f, "code"),
            Self::Environment(name) => write!(f, "environment {name}"),
            Self::ConfigFile(p) => write!(f, "config file {}", p.display()),
            Self::CredentialsFile(p) => write!(f, "credentials file {}", p.display()),
            Self::Default => write!(f, "default"),
        }
    }
}

impl From<&CredentialSource> for Source {
    fn from(source: &CredentialSource) -> Self {
        match source {
            CredentialSource::Environment => Self::Environment("AVISO_TOKEN or AVISO_USERNAME"),
            CredentialSource::ConfigFile(p) => Self::ConfigFile(p.clone()),
            CredentialSource::CredentialsFile(p) => Self::CredentialsFile(p.clone()),
        }
    }
}

/// A value together with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sourced<T> {
    /// The value the client will use.
    pub value: T,
    /// Where it came from.
    pub source: Source,
}

impl<T> Sourced<T> {
    fn new(value: T, source: Source) -> Self {
        Self { value, source }
    }
}

/// The credential a client will send, described without the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedAuth {
    /// The kind of credential: `bearer`, `basic`, `anonymous` when the
    /// caller asked for none, `chain` for a [`Chain`](crate::auth::Chain),
    /// or a custom provider's own name. A config-file provider that is
    /// mid-refresh reports `file`.
    pub kind: &'static str,
    /// Where it came from.
    pub source: Source,
    /// Set when the credential was found rather than named and the address
    /// is plain `http` on a host that is not loopback: the client refuses to
    /// send it, and this says so.
    pub refused: Option<String>,
}

/// Everything a client's connection depends on, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedSettings {
    /// The server address with any `user:password@` removed, or `None` when
    /// none of code, `AVISO_BASE_URL` and the config file supplied one.
    pub base_url: Option<Sourced<String>>,
    /// Per-request timeout.
    pub timeout: Sourced<Option<Duration>>,
    /// Expected heartbeat interval on watch streams.
    pub heartbeat_interval: Sourced<Option<Duration>>,
    /// Extra root certificates to trust.
    pub ca_bundle: Sourced<Vec<PathBuf>>,
    /// Whether certificate validation is off.
    pub danger_accept_invalid_certs: Sourced<bool>,
    /// The credential, or `None` when no source supplied one and the caller
    /// did not ask for anonymous access. An explicit request for no
    /// credential is reported as kind `anonymous` from [`Source::Code`].
    pub auth: Option<ResolvedAuth>,
    /// The config file that was read, if one existed.
    pub config_file: Option<PathBuf>,
    /// The credentials file that was consulted, if any.
    pub credentials_file: Option<PathBuf>,
}

impl std::fmt::Display for ResolvedSettings {
    /// One setting per line, value then source, aligned for reading. Nothing
    /// in it is a secret, so it can go into a log or a ticket as it is.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let line = |f: &mut std::fmt::Formatter<'_>, name: &str, value: &str, source: &str| {
            writeln!(f, "{name:<28}{value:<40}({source})")
        };
        match &self.base_url {
            Some(url) => line(f, "base_url", &url.value, &url.source.to_string())?,
            None => line(
                f,
                "base_url",
                "none",
                "not set in code, AVISO_BASE_URL or the config file",
            )?,
        }
        match &self.auth {
            Some(auth) => {
                let source = match &auth.refused {
                    Some(why) => format!("{}; {why}", auth.source),
                    None => auth.source.to_string(),
                };
                line(f, "auth", auth.kind, &source)?;
            }
            None => line(f, "auth", "none", "no credential found")?,
        }
        let seconds = |d: Option<Duration>| {
            d.map_or_else(|| "none".to_string(), |d| format!("{}s", d.as_secs_f64()))
        };
        line(
            f,
            "timeout",
            &seconds(self.timeout.value),
            &self.timeout.source.to_string(),
        )?;
        line(
            f,
            "heartbeat_interval",
            &seconds(self.heartbeat_interval.value),
            &self.heartbeat_interval.source.to_string(),
        )?;
        let bundle = self
            .ca_bundle
            .value
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        line(
            f,
            "ca_bundle",
            if bundle.is_empty() { "none" } else { &bundle },
            &self.ca_bundle.source.to_string(),
        )?;
        line(
            f,
            "danger_accept_invalid_certs",
            &self.danger_accept_invalid_certs.value.to_string(),
            &self.danger_accept_invalid_certs.source.to_string(),
        )
    }
}

/// What the caller supplied in code. Anything `None` is looked up. Fill it
/// with struct update syntax over [`CodeInputs::default`], so a field added
/// later needs no change at the call site.
#[derive(Debug, Clone, Default)]
pub struct CodeInputs {
    /// Server address.
    pub base_url: Option<String>,
    /// Per-request timeout.
    pub timeout: Option<Duration>,
    /// Expected heartbeat interval.
    pub heartbeat_interval: Option<Duration>,
    /// Whether certificate validation is off.
    pub danger_accept_invalid_certs: Option<bool>,
    /// The kind of a credential the caller named, when it named one. A named
    /// credential is never refused.
    pub auth_kind: Option<&'static str>,
}

/// Whether the `AVISO_BASE_URL` environment variable may supply the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvAddress {
    /// Consult it, after a value passed in code and before the config file.
    Read,
    /// Leave it alone; the address comes from code or the file.
    Ignore,
}

/// Resolves settings from `inputs`, the environment (the address only when
/// `env_address` is [`EnvAddress::Read`]), the config file at
/// `paths.config_file` (already read into `paths.config_content` when the
/// caller has done so) and the credentials file at `paths.credentials_file`.
///
/// The credential search runs only when `inputs.auth_kind` is `None`; a
/// found credential is checked against the resolved address and marked
/// refused rather than dropped, so a dump can say why it is not being sent.
///
/// # Errors
///
/// Returns [`crate::ClientError::Config`] when the config file or the
/// credentials file exists but cannot be read or parsed, and
/// [`crate::ClientError::Auth`] when a credential source is present but
/// unusable, such as a username with no password.
pub fn resolve(
    inputs: &CodeInputs,
    paths: &DiscoveryPaths,
    env_address: EnvAddress,
) -> crate::Result<Resolution> {
    let mut paths = paths.clone();
    let loaded = ClientSettings::read_default(paths.config_file.as_deref())?;
    // The file is read once. Its text goes to the credential search, so the
    // credential cannot come from a newer file than the settings. When there
    // was no file, the search skips that tier rather than probe the path
    // again and find something that appeared since.
    let (settings, config_file) = if let Some(loaded) = loaded {
        paths.config_file = Some(loaded.path.clone());
        paths.config_content = Some(loaded.content);
        (loaded.settings, Some(loaded.path))
    } else {
        paths.config_file = None;
        (ClientSettings::default(), None)
    };
    let file_source = |value_present: bool| match (&config_file, value_present) {
        (Some(p), true) => Source::ConfigFile(p.clone()),
        _ => Source::Default,
    };

    let base_url = if let Some(url) = &inputs.base_url {
        Some(Sourced::new(url.clone(), Source::Code))
    } else if let Some(url) = (env_address == EnvAddress::Read)
        .then(|| env_var("AVISO_BASE_URL"))
        .flatten()
    {
        Some(Sourced::new(url, Source::Environment("AVISO_BASE_URL")))
    } else {
        settings
            .base_url
            .clone()
            .map(|url| Sourced::new(url, file_source(true)))
    };

    let timeout = match inputs.timeout {
        Some(t) => Sourced::new(Some(t), Source::Code),
        None => Sourced::new(settings.timeout, file_source(settings.timeout.is_some())),
    };
    let heartbeat_interval = match inputs.heartbeat_interval {
        Some(h) => Sourced::new(Some(h), Source::Code),
        None => Sourced::new(
            settings.heartbeat_interval,
            file_source(settings.heartbeat_interval.is_some()),
        ),
    };
    let ca_bundle_paths = settings.ca_bundle.clone();
    let ca_bundle = Sourced::new(
        settings.ca_bundle.clone(),
        file_source(!settings.ca_bundle.is_empty()),
    );
    let danger_accept_invalid_certs = match inputs.danger_accept_invalid_certs {
        Some(v) => Sourced::new(v, Source::Code),
        None => Sourced::new(
            settings.danger_accept_invalid_certs,
            file_source(settings.danger_accept_invalid_certs),
        ),
    };

    let (auth, found) = resolve_auth(inputs, &paths, base_url.as_ref().map(|u| u.value.as_str()))?;

    let raw_base_url = base_url.as_ref().map(|url| url.value.clone());
    let settings = ResolvedSettings {
        base_url: base_url.map(|url| Sourced::new(display_address(&url.value), url.source)),
        timeout,
        heartbeat_interval,
        ca_bundle,
        danger_accept_invalid_certs,
        auth,
        config_file,
        credentials_file: paths.credentials_file.clone(),
    };
    Ok(Resolution {
        settings,
        raw_base_url,
        file_settings: ClientSettings {
            ca_bundle: ca_bundle_paths,
            ..ClientSettings::default()
        },
        found,
    })
}

/// The result of [`resolve`]: the report, plus what the builder needs and a
/// dump must not show. `Debug` prints the report only.
#[non_exhaustive]
pub struct Resolution {
    /// The report.
    pub settings: ResolvedSettings,
    /// The winning address exactly as written, credentials and all, for the
    /// builder. `None` when no source supplied one.
    pub(crate) raw_base_url: Option<String>,
    /// The certificate paths from the file, for the builder to load.
    pub(crate) file_settings: ClientSettings,
    /// The found credential, for the builder to attach with its source.
    pub(crate) found: Option<Discovered>,
}

impl std::fmt::Debug for Resolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolution")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// Describes the credential the client will send. A credential named in
/// code is reported as such and never searched for; otherwise the search
/// runs, and a found credential headed for a public plaintext address is
/// marked refused rather than dropped, so a dump can say why.
fn resolve_auth(
    inputs: &CodeInputs,
    paths: &DiscoveryPaths,
    base_url: Option<&str>,
) -> crate::Result<(Option<ResolvedAuth>, Option<Discovered>)> {
    if let Some(kind) = inputs.auth_kind {
        return Ok((
            Some(ResolvedAuth {
                kind,
                source: Source::Code,
                refused: None,
            }),
            None,
        ));
    }
    let Some(found) = crate::auth::discover_with(paths)? else {
        return Ok((None, None));
    };
    let refused = base_url
        .filter(|url| crate::auth::is_public_plaintext(url))
        .map(|url| {
            format!(
                "refused: {} is plain http on a host that is not loopback, so \
                 the client will not be built. Use https, or name the \
                 credential in code to send it anyway.",
                crate::auth::url_without_userinfo(url)
            )
        });
    Ok((
        Some(ResolvedAuth {
            kind: found.provider().kind(),
            source: Source::from(found.source()),
            refused,
        }),
        Some(found),
    ))
}

/// The address as a dump shows it: parsed and re-serialised with any
/// `user:password@` removed and the trailing slash the built client will
/// have, so it reads the same however it was written. A value that does
/// not parse is shown as a placeholder, since the part that broke it may
/// sit next to a password.
fn display_address(url: &str) -> String {
    url::Url::parse(url).map_or_else(
        |_| "<unparseable url>".to_string(),
        |mut parsed| {
            if !parsed.path().ends_with('/') {
                let normalized = format!("{}/", parsed.path());
                parsed.set_path(&normalized);
            }
            crate::client::display_url(&parsed)
        },
    )
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
