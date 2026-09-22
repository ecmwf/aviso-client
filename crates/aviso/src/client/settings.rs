// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Client settings read from the aviso config file.
//!
//! The `aviso` binary reads `~/.config/aviso/config.yaml`. A library user who
//! has set that file up for the binary should not have to repeat its contents
//! in every script, so [`AvisoClientBuilder::from_file`] reads the settings a
//! client needs from the same file: `base_url`, `timeout`,
//! `heartbeat_interval`, `tls`, and, through credential discovery, `auth`.
//!
//! The file also holds sections that describe what the binary should do
//! rather than how to connect, such as `listeners` and `state_file`. A library
//! caller expresses that in code, so unknown top-level keys are ignored here.
//! Inside a section this module does read, an unknown key is an error, the
//! same rule the `auth:` block already follows.
//!
//! Valid:
//!
//! ```yaml
//! base_url: https://aviso.example.org
//! timeout: 30s
//! tls:
//!   ca_bundle: [/etc/aviso/internal-ca.pem]
//! listeners: []          # ignored: not a client setting
//! ```
//!
//! Invalid, because `tls` is a section this module reads:
//!
//! ```yaml
//! tls:
//!   ca_bundel: [/etc/aviso/internal-ca.pem]
//! ```
//!
//! [`AvisoClientBuilder::from_file`]: crate::AvisoClientBuilder::from_file

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::ClientError;

/// The client-relevant part of a config file.
///
/// Every field is optional. A missing file yields [`Self::default`], which
/// sets nothing, so a builder made from it behaves like one made by hand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientSettings {
    /// Server address.
    pub base_url: Option<String>,
    /// Per-request timeout.
    pub timeout: Option<Duration>,
    /// Expected heartbeat interval on watch streams.
    pub heartbeat_interval: Option<Duration>,
    /// PEM files with extra root certificates to trust. Relative paths are
    /// resolved against the config file's directory, so a file that says
    /// `ca_bundle: [internal-ca.pem]` means the PEM beside it, wherever the
    /// process was started from.
    pub ca_bundle: Vec<PathBuf>,
    /// Whether to skip TLS certificate validation. Off unless the file says so.
    pub danger_accept_invalid_certs: bool,
}

impl ClientSettings {
    /// Reads the settings from a file.
    ///
    /// A path that does not exist is an error: the caller named it, so its
    /// absence is a mistake. Callers that want "absent is fine" semantics for a
    /// default location use [`Self::from_default_path`].
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when the file cannot be read or parsed,
    /// or when a section this module reads contains an unknown key.
    pub fn from_path(path: impl AsRef<Path>) -> crate::Result<Self> {
        Ok(Self::read(path.as_ref())?.settings)
    }

    /// Reads and parses a file, keeping the text it was parsed from.
    ///
    /// Callers that also need the `auth:` block hand that text to credential
    /// discovery, so the credential and the settings come from one read of the
    /// file rather than two that could straddle a replacement.
    ///
    /// # Errors
    ///
    /// As [`Self::from_path`].
    pub fn read(path: &Path) -> crate::Result<LoadedSettings> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ClientError::Config(format!("read config file {}: {e}", path.display()))
        })?;
        let settings = Self::parse(&content, path)?;
        Ok(LoadedSettings {
            settings,
            content,
            path: path.to_path_buf(),
        })
    }

    /// Like [`Self::read`], for the default location; `Ok(None)` when there is
    /// no file there.
    ///
    /// # Errors
    ///
    /// As [`Self::from_default_path`].
    pub fn read_default(path_override: Option<&Path>) -> crate::Result<Option<LoadedSettings>> {
        let path = match path_override {
            Some(p) => p.to_path_buf(),
            None => match crate::auth::DiscoveryPaths::from_env().config_file {
                Some(p) => p,
                None => return Ok(None),
            },
        };
        match std::fs::symlink_metadata(&path) {
            Ok(_) => Self::read(&path).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ClientError::Config(format!(
                "read config file {}: {e}",
                path.display()
            ))),
        }
    }

    /// Reads the settings from the default location, or returns defaults when
    /// there is no file there.
    ///
    /// The location is `~/.config/aviso/config.yaml`, or the path in
    /// `AVISO_CLIENT_CONFIG_FILE`. Only a genuinely missing file yields
    /// defaults: a file that exists but cannot be read is still an error, so a
    /// typo or a permissions problem is reported rather than silently treated
    /// as "no settings".
    ///
    /// # Errors
    ///
    /// As [`Self::from_path`], except that a missing file is not an error.
    pub fn from_default_path() -> crate::Result<Self> {
        Ok(Self::read_default(None)?
            .map(|loaded| loaded.settings)
            .unwrap_or_default())
    }

    /// Parses settings from text. `path` is used for messages and to resolve
    /// relative certificate paths.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when the text is not valid YAML or a
    /// section this module reads contains an unknown key.
    pub fn parse(content: &str, path: &Path) -> crate::Result<Self> {
        let doc: Doc = serde_norway::from_str(content).map_err(|e| {
            ClientError::Config(format!("parse config file {}: {e}", path.display()))
        })?;
        let base_dir = path.parent().filter(|p| !p.as_os_str().is_empty());
        let tls = doc.tls.unwrap_or_default();
        Ok(Self {
            base_url: doc.base_url,
            timeout: doc.timeout,
            heartbeat_interval: doc.heartbeat_interval,
            ca_bundle: tls
                .ca_bundle
                .into_iter()
                .map(|p| match base_dir {
                    Some(dir) if p.is_relative() => dir.join(p),
                    _ => p,
                })
                .collect(),
            danger_accept_invalid_certs: tls.danger_accept_invalid_certs,
        })
    }
}

/// Reads a PEM file and checks it holds at least one certificate.
///
/// `reqwest::Certificate::from_pem` accepts any bytes and simply yields no
/// certificates for text that has no `BEGIN CERTIFICATE` block, so a mistyped
/// or empty file would add nothing to the trust store and TLS would fail
/// later with a message about the server instead of the file. Checking for a
/// block here turns that into an error that names the file.
///
/// # Errors
///
/// Returns [`ClientError::Config`] when the file cannot be read, holds no
/// certificate block, or does not parse.
pub fn read_ca_bundle(path: &Path) -> crate::Result<reqwest::Certificate> {
    let pem = std::fs::read(path)
        .map_err(|e| ClientError::Config(format!("read ca_bundle file {}: {e}", path.display())))?;
    if !pem
        .windows(b"-----BEGIN CERTIFICATE-----".len())
        .any(|w| w == b"-----BEGIN CERTIFICATE-----")
    {
        return Err(ClientError::Config(format!(
            "ca_bundle file {} contains no certificate: expected at least one \
             PEM block starting with -----BEGIN CERTIFICATE-----",
            path.display()
        )));
    }
    reqwest::Certificate::from_pem(&pem).map_err(|e| {
        ClientError::Config(format!(
            "ca_bundle file {} is not a PEM certificate: {e}",
            path.display()
        ))
    })
}

/// Settings together with the text they were parsed from and where it came
/// from. Returned by [`ClientSettings::read`]; the text feeds credential
/// discovery so both read one snapshot. No `Debug`: the text may hold a token.
#[non_exhaustive]
pub struct LoadedSettings {
    /// The parsed settings.
    pub settings: ClientSettings,
    /// The exact file text.
    pub content: String,
    /// The file it was read from.
    pub path: PathBuf,
}

/// Only the keys a client needs are modelled. Unknown top-level keys, `auth:`
/// among them, are ignored; credential discovery reads `auth:` from the same
/// text and owns its shape.
#[derive(Deserialize)]
struct Doc {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default, with = "humantime_serde::option")]
    timeout: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    heartbeat_interval: Option<Duration>,
    #[serde(default)]
    tls: Option<TlsDoc>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TlsDoc {
    #[serde(default)]
    ca_bundle: Vec<PathBuf>,
    #[serde(default)]
    danger_accept_invalid_certs: bool,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on fixture setup are the expected diagnostics"
)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ClientSettings {
        ClientSettings::parse(text, Path::new("/etc/aviso/config.yaml")).unwrap()
    }

    #[test]
    fn reads_every_client_setting() {
        let s = parse(
            "base_url: https://aviso.example.org\n\
             timeout: 30s\n\
             heartbeat_interval: 45s\n\
             tls:\n  ca_bundle: [/certs/a.pem]\n  danger_accept_invalid_certs: true\n",
        );

        assert_eq!(s.base_url.as_deref(), Some("https://aviso.example.org"));
        assert_eq!(s.timeout, Some(Duration::from_secs(30)));
        assert_eq!(s.heartbeat_interval, Some(Duration::from_secs(45)));
        assert_eq!(s.ca_bundle, vec![PathBuf::from("/certs/a.pem")]);
        assert!(s.danger_accept_invalid_certs);
    }

    #[test]
    fn an_empty_file_sets_nothing() {
        assert_eq!(parse(""), ClientSettings::default());
    }

    #[test]
    fn ignores_sections_that_belong_to_the_binary() {
        let s = parse(
            "base_url: https://aviso.example.org\n\
             state_file: /var/lib/aviso/state.json\n\
             listeners:\n  - event: mars\n    identifiers: {}\n\
             auth:\n  bearer_token: not-read-here\n",
        );

        assert_eq!(s.base_url.as_deref(), Some("https://aviso.example.org"));
    }

    #[test]
    fn rejects_an_unknown_key_inside_a_section_it_reads() {
        let error = ClientSettings::parse(
            "tls:\n  ca_bundel: [/certs/a.pem]\n",
            Path::new("/etc/aviso/config.yaml"),
        )
        .unwrap_err();

        assert!(matches!(error, ClientError::Config(_)), "got {error:?}");
        assert!(error.to_string().contains("ca_bundel"), "got {error}");
    }

    #[test]
    fn relative_certificate_paths_are_resolved_against_the_file() {
        let s = parse("tls:\n  ca_bundle: [internal-ca.pem, /abs/other.pem]\n");

        assert_eq!(
            s.ca_bundle,
            vec![
                PathBuf::from("/etc/aviso/internal-ca.pem"),
                PathBuf::from("/abs/other.pem"),
            ]
        );
    }

    #[test]
    fn debug_output_never_carries_the_auth_block() {
        let s = parse("auth:\n  bearer_token: super-secret-value\n");

        assert!(!format!("{s:?}").contains("super-secret-value"));
    }

    #[test]
    fn a_named_path_that_does_not_exist_is_an_error() {
        let dir = tempfile::tempdir().unwrap();

        let error = ClientSettings::from_path(dir.path().join("absent.yaml")).unwrap_err();

        assert!(matches!(error, ClientError::Config(_)), "got {error:?}");
    }

    #[test]
    fn from_path_reads_and_resolves_relative_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "tls:\n  ca_bundle: [ca.pem]\n").unwrap();

        let s = ClientSettings::from_path(&path).unwrap();

        assert_eq!(s.ca_bundle, vec![dir.path().join("ca.pem")]);
    }
}
