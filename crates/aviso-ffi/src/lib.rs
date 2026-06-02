//! Stable C ABI for [`aviso`], the client library for ECMWF's `aviso-server`
//! notification service, plus a hand-written header-only C++ facade
//! (`include/aviso.hpp`) over it.
//!
//! This crate is a peer consumer of the core `aviso` crate: it depends on
//! `aviso` and never the reverse, and the core carries no FFI machinery. The
//! generated C header is `include/aviso.h`; regenerate it with the
//! `gen-header` feature (see the crate README). The design rationale lives in
//! `plans/decisions.md` (ADR D21).
//!
//! # Boundary contract
//!
//! - Every fallible call returns an owning [`AvisoOutcome`]; inspect it, take
//!   any success value with the matching typed `take`, then free it.
//! - Handles are owning pointers, each freed by its `aviso_*_free`. Builder and
//!   request handles are consumed through a pointer-to-pointer that is nulled
//!   on consumption.
//! - Strings cross as UTF-8 `const char*`; JSON crosses as compact-JSON
//!   `const char*`.
//! - Blocking calls must not run on a thread already inside the runtime.
//! - Every entry point traps Rust panics and reports them as
//!   [`AvisoErrorKind::Panic`] rather than unwinding across the boundary.
//!
//! [`aviso`]: https://docs.rs/aviso

mod client;
mod error;
mod outcome;
mod watch;

pub use client::{AvisoClient, AvisoClientBuilder};
pub use error::{AvisoError, AvisoErrorKind};
pub use outcome::AvisoOutcome;
pub use watch::{AvisoNotification, AvisoWatch, AvisoWatchRequest};

use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use crate::error::OutcomeError;

/// The process-global multi-thread runtime shared by every client and call.
///
/// One runtime for the process (not one per client): the core client is
/// `Send + Sync + Clone`, so a per-client runtime buys no isolation and makes
/// freeing a client from a runtime thread unsafe. The runtime lives until the
/// process exits.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        #[allow(
            clippy::expect_used,
            reason = "fatal init: a process that cannot build a tokio runtime cannot serve any \
                      call; the panic is trapped by the per-call catch_unwind boundary and \
                      reported as AvisoErrorKind::Panic"
        )]
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("aviso-ffi: failed to build the tokio runtime")
    })
}

/// Returns an `InvalidUsage` error when called from a thread already inside the
/// runtime (a blocking call from within a callback would otherwise panic with
/// "cannot start a runtime from within a runtime").
pub(crate) fn reject_blocking_on_runtime() -> Option<OutcomeError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        Some(error::invalid_usage(
            "blocking aviso call from inside the runtime (a callback runs on a runtime thread) \
             is not allowed",
        ))
    } else {
        None
    }
}

/// Runs `f`, trapping any panic and reporting it as an [`AvisoErrorKind::Panic`]
/// outcome instead of unwinding across the C boundary.
pub(crate) fn guard_outcome(f: impl FnOnce() -> *mut AvisoOutcome) -> *mut AvisoOutcome {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(outcome) => outcome,
        Err(_) => error::panic_outcome(),
    }
}

/// Runs `f`, returning `default` if it panics. For entry points whose return
/// type cannot carry a structured error (handle constructors, void setters,
/// frees).
pub(crate) fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

/// Returns the library version as a static, NUL-terminated string. The pointer
/// is valid for the life of the process and must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn aviso_version() -> *const c_char {
    static VERSION: OnceLock<CString> = OnceLock::new();
    VERSION
        .get_or_init(|| CString::new(aviso::VERSION).unwrap_or_default())
        .as_ptr()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
mod tests {
    use super::*;
    use crate::client::{aviso_client_builder_build, aviso_client_builder_new};
    use crate::outcome;
    use std::ffi::{CStr, CString};

    #[test]
    fn version_is_non_empty() {
        let ptr = aviso_version();
        assert!(!ptr.is_null());
        // SAFETY: aviso_version returns a static NUL-terminated string.
        let version = unsafe { CStr::from_ptr(ptr) }.to_str().expect("utf8");
        assert!(!version.is_empty());
    }

    #[test]
    fn builder_build_with_invalid_base_url_reports_config_error() {
        // An unparseable base_url surfaces a Config error from the core,
        // exercising the outcome + error mapping end to end.
        // SAFETY: a NUL-terminated literal.
        let base = CString::new("not a url").expect("cstring");
        // SAFETY: valid C string pointer.
        let mut builder = unsafe { aviso_client_builder_new(base.as_ptr()) };
        assert!(!builder.is_null());
        // SAFETY: builder is a live handle pointer.
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        assert!(!outcome.is_null());
        // The build consumed the builder and nulled our pointer.
        assert!(builder.is_null());
        // SAFETY: outcome is a live outcome.
        let ok = unsafe { outcome::aviso_outcome_is_ok(outcome) };
        assert!(!ok, "an invalid base_url must not build a client");
        // SAFETY: outcome is a live outcome with an error.
        let err = unsafe { outcome::aviso_outcome_error(outcome) };
        assert!(!err.is_null());
        // SAFETY: err points into the live outcome.
        let kind = unsafe { (*err).kind };
        assert_eq!(kind, AvisoErrorKind::Config);
        // SAFETY: outcome is live and owned here.
        unsafe { outcome::aviso_outcome_free(outcome) };
    }

    #[test]
    fn null_base_url_reports_invalid_input_at_build() {
        // SAFETY: passing null is explicitly supported and remembered.
        let mut builder = unsafe { aviso_client_builder_new(std::ptr::null()) };
        assert!(!builder.is_null());
        // SAFETY: live handle pointer.
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        // SAFETY: live outcome.
        let err = unsafe { outcome::aviso_outcome_error(outcome) };
        assert!(!err.is_null());
        // SAFETY: err points into the live outcome.
        assert_eq!(unsafe { (*err).kind }, AvisoErrorKind::InvalidInput);
        // SAFETY: live, owned outcome.
        unsafe { outcome::aviso_outcome_free(outcome) };
    }
}
