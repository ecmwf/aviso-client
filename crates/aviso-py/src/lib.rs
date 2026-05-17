//! `PyO3` bindings for [`aviso`].
//!
//! Currently an `rlib` re-exporting the core crate's version. The Python
//! extension surface (a `cdylib` with the actual `PyO3` bindings) is added
//! when the corresponding work lands; the rest of the package metadata is
//! already wired so adding the bindings is a code-only change.

#![forbid(unsafe_code)]

/// Version string of the underlying [`aviso`] core crate.
///
/// The binding crate and the core crate share a workspace version, so a
/// single re-export keeps the surface honest: there is one version,
/// sourced from `Cargo.toml`.
pub const VERSION: &str = aviso::VERSION;
