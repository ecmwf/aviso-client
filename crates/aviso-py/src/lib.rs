//! `PyO3` bindings for [`aviso`].
//!
//! Phase 0 ships only the scaffold: an `rlib` re-exporting the core
//! version. Phase 5 converts this crate to a `cdylib`, adds the `PyO3`
//! dependency, and exposes the actual bindings.

#![forbid(unsafe_code)]

/// Version string of the underlying [`aviso`] core crate.
///
/// The binding crate and the core crate share a workspace version, so a
/// single re-export keeps the surface honest: there is one version,
/// sourced from `Cargo.toml`.
pub const VERSION: &str = aviso::VERSION;
