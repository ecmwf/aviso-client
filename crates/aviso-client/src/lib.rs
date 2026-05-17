//! Core client library for [`aviso-server`], ECMWF's notification service for
//! data-driven workflows.
//!
//! Phase 0 ships only the workspace scaffold. The public surface is empty by
//! design; behaviour lands phase by phase, tracked in
//! `docs/src/internals/decisions.md`.
//!
//! [`aviso-server`]: https://github.com/ecmwf/aviso-server

#![forbid(unsafe_code)]

/// Version string of the `aviso-client` crate, sourced from Cargo metadata.
///
/// Exposed so the CLI and the Python extension can render a single, consistent
/// version line without re-reading `CARGO_PKG_VERSION` themselves.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_non_empty() {
        assert!(!VERSION.is_empty());
    }
}
