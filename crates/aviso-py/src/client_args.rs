// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The constructor arguments the two clients share, and how they become a
//! client and a report.
//!
//! Every argument is optional. `None` means "look it up": the environment,
//! then the aviso config file, then the default. The lookup runs once per
//! constructor, through `aviso::resolve`, and its result is used both to
//! build the client and to fill `client.config`, so the report cannot drift
//! from what was built.

use aviso::resolve::{CodeInputs, EnvAddress, Resolution};
use aviso::{AvisoClient, AvisoClientBuilder};
use pyo3::prelude::*;

use crate::auth::{extract_provider, is_anonymous};
use crate::config::PyResolvedConfig;
use crate::error::{duration_from_seconds, map_client_error};
use crate::state_stores::extract_store;

/// The `auth` argument: `None` means "look for a credential", the
/// `Anonymous()` marker means "send nothing", and anything else is used as
/// given, which is the caller choosing where it goes.
#[derive(Default)]
pub(crate) struct ClientArgs<'a> {
    pub(crate) base_url: Option<String>,
    pub(crate) auth: Option<&'a Bound<'a, PyAny>>,
    pub(crate) timeout: Option<f64>,
    pub(crate) user_agent: Option<String>,
    pub(crate) state_store: Option<&'a Bound<'a, PyAny>>,
    pub(crate) heartbeat_interval: Option<f64>,
    pub(crate) danger_accept_invalid_certs: Option<bool>,
    pub(crate) flush_cursor_on_exit: Option<bool>,
}

/// A client together with the report of what it was built from.
pub(crate) struct Built {
    pub(crate) client: AvisoClient,
    pub(crate) config: Py<PyResolvedConfig>,
}

impl ClientArgs<'_> {
    /// What the resolver needs to know about the caller's inputs.
    fn code_inputs(&self) -> PyResult<CodeInputs> {
        let auth_kind = match self.auth {
            Some(obj) if is_anonymous(obj) => Some("anonymous"),
            Some(obj) => Some(extract_provider(obj)?.kind()),
            None => None,
        };
        Ok(CodeInputs {
            base_url: self.base_url.clone(),
            timeout: self
                .timeout
                .map(|secs| duration_from_seconds("timeout", secs))
                .transpose()?,
            heartbeat_interval: self
                .heartbeat_interval
                .map(|secs| duration_from_seconds("heartbeat_interval", secs))
                .transpose()?,
            danger_accept_invalid_certs: self.danger_accept_invalid_certs,
            auth_kind,
        })
    }

    /// Resolves against the default file, with `AVISO_BASE_URL` taking part.
    /// This is what `AvisoClient()` does.
    pub(crate) fn build(&self, py: Python<'_>) -> PyResult<Built> {
        let resolution = self.resolve(
            py,
            &aviso::auth::DiscoveryPaths::from_env(),
            EnvAddress::Read,
        )?;
        self.finish(py, resolution)
    }

    /// Resolves against a named file, or the default one when `path` is
    /// `None`, with the address read from the file only. This is what
    /// `AvisoClient.from_file()` does. A named file must exist.
    pub(crate) fn build_from_file(
        &self,
        py: Python<'_>,
        path: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Built> {
        let mut paths = aviso::auth::DiscoveryPaths::from_env();
        if let Some(p) = path {
            // The same convention as every other path this binding accepts:
            // os.fspath, then expanduser, so "~/aviso.yaml" means the home
            // directory. The resolver treats a missing path as no file, so
            // the existence check happens here, by reading it.
            let path = crate::paths::normalize_path(py, p)?;
            let loaded = aviso::ClientSettings::read(&path).map_err(|e| map_client_error(py, e))?;
            paths.config_file = Some(loaded.path);
            paths.config_content = Some(loaded.content);
        }
        let resolution = self.resolve(py, &paths, EnvAddress::Ignore)?;
        self.finish(py, resolution)
    }

    /// The report alone, without building. What `pyaviso.resolve_config()`
    /// returns.
    pub(crate) fn report(&self, py: Python<'_>) -> PyResult<Py<PyResolvedConfig>> {
        let resolution = self.resolve(
            py,
            &aviso::auth::DiscoveryPaths::from_env(),
            EnvAddress::Read,
        )?;
        crate::config::to_py(py, &resolution.settings)
    }

    fn resolve(
        &self,
        py: Python<'_>,
        paths: &aviso::auth::DiscoveryPaths,
        env_address: EnvAddress,
    ) -> PyResult<Resolution> {
        aviso::resolve::resolve(&self.code_inputs()?, paths, env_address)
            .map_err(|e| map_client_error(py, e))
    }

    /// Turns one resolution into the client and its report.
    fn finish(&self, py: Python<'_>, resolution: Resolution) -> PyResult<Built> {
        let config = crate::config::to_py(py, &resolution.settings)?;
        let builder =
            AvisoClientBuilder::from_resolution(resolution).map_err(|e| map_client_error(py, e))?;
        let client = self
            .apply(builder)?
            .build()
            .map_err(|e| map_client_error(py, e))?;
        Ok(Built { client, config })
    }

    /// Applies what the resolver does not carry: the credential object, the
    /// user agent, the state store and the flush flag. The address and the
    /// timeouts passed in code are already in the builder.
    fn apply(&self, mut builder: AvisoClientBuilder) -> PyResult<AvisoClientBuilder> {
        if let Some(obj) = self.auth {
            builder = if is_anonymous(obj) {
                builder.anonymous()
            } else {
                builder.auth(extract_provider(obj)?)
            };
        }
        if let Some(ua) = &self.user_agent {
            builder = builder.user_agent(ua.clone());
        }
        if let Some(store) = self.state_store {
            builder = builder.state_store(extract_store(store)?);
        }
        if let Some(v) = self.flush_cursor_on_exit {
            builder = builder.flush_cursor_on_exit(v);
        }
        Ok(builder)
    }
}

/// Reports the settings a client built with these arguments would use, and
/// where each came from, without connecting. The same arguments as the
/// `AvisoClient` constructor.
#[pyfunction]
#[pyo3(signature = (*, base_url = None, auth = None, timeout = None,
                      heartbeat_interval = None, danger_accept_invalid_certs = None))]
pub(crate) fn resolve_config(
    py: Python<'_>,
    base_url: Option<String>,
    auth: Option<&Bound<'_, PyAny>>,
    timeout: Option<f64>,
    heartbeat_interval: Option<f64>,
    danger_accept_invalid_certs: Option<bool>,
) -> PyResult<Py<PyResolvedConfig>> {
    ClientArgs {
        base_url,
        auth,
        timeout,
        heartbeat_interval,
        danger_accept_invalid_certs,
        ..ClientArgs::default()
    }
    .report(py)
}
