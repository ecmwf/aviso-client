//! `PyO3` bindings for [`aviso_client`].
//!
//! Phase 0 ships only the scaffold: an `rlib` that re-exports the version
//! constant. Phase 5 converts this crate to a `cdylib`, adds the `PyO3`
//! dependency, and exposes the actual bindings.

#![forbid(unsafe_code)]

/// Version string of the bindings crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version string of the underlying [`aviso_client`] core crate.
pub const CORE_VERSION: &str = aviso_client::VERSION;

#[cfg(test)]
mod tests {
    use super::{CORE_VERSION, VERSION};

    #[test]
    fn versions_are_non_empty() {
        assert!(!VERSION.is_empty());
        assert!(!CORE_VERSION.is_empty());
    }
}
