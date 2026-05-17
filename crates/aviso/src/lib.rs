//! Core client library for [`aviso-server`], ECMWF's notification service for
//! data-driven workflows.
//!
//! Phase 0 ships only the workspace scaffold. The public surface is empty by
//! design; behaviour lands phase by phase, tracked in
//! `docs/src/internals/decisions.md`.
//!
//! [`aviso-server`]: https://github.com/ecmwf/aviso-server

#![forbid(unsafe_code)]

/// Version string of the `aviso` crate, sourced from Cargo metadata.
///
/// Exposed so the CLI and the Python extension can render a single, consistent
/// version line without re-reading `CARGO_PKG_VERSION` themselves.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
