//! Client and client-builder handles, and the blocking verbs over them.

use std::ffi::{CStr, c_char};
use std::ptr;
use std::sync::Arc;

use aviso::auth::Basic;

use crate::error::{self, OutcomeError};
use crate::outcome::AvisoOutcome;
use crate::{guard, guard_outcome, reject_blocking_on_runtime, runtime};

/// Opaque client handle. Wraps the core client (cheap to clone, shares the
/// connection pool and auth state).
pub struct AvisoClient {
    pub(crate) inner: aviso::AvisoClient,
}

/// Opaque client-builder handle. Setters mutate it in place; the first error
/// (a bad argument or an auth-construction failure) is remembered and surfaced
/// at [`aviso_client_builder_build`].
pub struct AvisoClientBuilder {
    inner: Option<aviso::AvisoClientBuilder>,
    error: Option<OutcomeError>,
}

impl AvisoClientBuilder {
    fn apply(&mut self, f: impl FnOnce(aviso::AvisoClientBuilder) -> aviso::AvisoClientBuilder) {
        if let Some(inner) = self.inner.take() {
            self.inner = Some(f(inner));
        }
    }
}

/// Reads a borrowed string argument, or `None` when null or not UTF-8.
///
/// # Safety
///
/// `ptr`, when non-null, must point to a NUL-terminated C string valid for the
/// duration of the call.
unsafe fn cstr_opt<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Creates a client builder for `base_url`. Always returns a non-null handle;
/// a null or non-UTF-8 `base_url` is remembered and reported at build time.
/// Free an abandoned builder with [`aviso_client_builder_free`].
///
/// # Safety
///
/// `base_url`, when non-null, must point to a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_new(
    base_url: *const c_char,
) -> *mut AvisoClientBuilder {
    guard(ptr::null_mut(), || {
        let mut builder = AvisoClientBuilder {
            inner: Some(aviso::AvisoClient::builder()),
            error: None,
        };
        match unsafe { cstr_opt(base_url) } {
            Some(url) => builder.apply(|b| b.base_url(url)),
            None => {
                builder.error = Some(error::invalid_input(
                    "base_url must be a non-null UTF-8 string",
                ));
            }
        }
        Box::into_raw(Box::new(builder))
    })
}

/// Sets HTTP Basic credentials on the builder. A null or non-UTF-8 argument, or
/// a credential-construction failure, is remembered and reported at build time.
///
/// # Safety
///
/// `builder` must be a live builder handle from [`aviso_client_builder_new`].
/// `username` and `password`, when non-null, must be NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_basic_auth(
    builder: *mut AvisoClientBuilder,
    username: *const c_char,
    password: *const c_char,
) {
    guard((), || {
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            return;
        };
        if builder.error.is_some() {
            return;
        }
        let (Some(user), Some(pass)) =
            (unsafe { cstr_opt(username) }, unsafe { cstr_opt(password) })
        else {
            builder.error = Some(error::invalid_input(
                "basic_auth username and password must be non-null UTF-8 strings",
            ));
            return;
        };
        match Basic::new(user, pass) {
            Ok(basic) => builder.apply(|b| b.auth(Arc::new(basic))),
            Err(err) => builder.error = Some(error::map_error(&err)),
        }
    });
}

/// Builds the client, consuming the builder. On entry the builder is taken and
/// the caller's pointer is set to null (so a later free is a safe no-op), even
/// when the build fails. Returns an outcome carrying the client (retrieve it
/// with `aviso_outcome_take_client`) or a structured error.
///
/// # Safety
///
/// `builder` must point to a builder-handle pointer. `*builder`, when non-null,
/// must be a live handle from [`aviso_client_builder_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_build(
    builder: *mut *mut AvisoClientBuilder,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        if builder.is_null() {
            return error::invalid_input("builder handle pointer must not be null").into_outcome();
        }
        let slot = unsafe { &mut *builder };
        if slot.is_null() {
            return error::invalid_input("builder is null or already consumed").into_outcome();
        }
        // Take ownership and null the caller's pointer before doing any work.
        let owned = unsafe { Box::from_raw(*slot) };
        *slot = ptr::null_mut();

        if let Some(err) = owned.error {
            return err.into_outcome();
        }
        let Some(inner) = owned.inner else {
            return error::internal("builder was already consumed").into_outcome();
        };
        match inner.build() {
            Ok(client) => AvisoOutcome::client(Box::new(AvisoClient { inner: client })).into_raw(),
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}

/// Frees an abandoned client builder. Builders consumed by
/// [`aviso_client_builder_build`] are already freed; calling this on the
/// nulled-out pointer is a safe no-op.
///
/// # Safety
///
/// `builder`, when non-null, must be a live handle from
/// [`aviso_client_builder_new`] that was not consumed by a build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_free(builder: *mut AvisoClientBuilder) {
    if builder.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(builder) });
}

/// Frees a client. A null pointer is a no-op.
///
/// # Safety
///
/// `client`, when non-null, must be a live handle and not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_free(client: *mut AvisoClient) {
    if client.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(client) });
}

/// Fetches the schema catalog (`GET /api/v1/schema`) and returns it as a
/// compact-JSON string success value (retrieve it with
/// `aviso_outcome_take_string`), or a structured error.
///
/// This call blocks. It must not be called from a thread already inside the
/// runtime (for example a watch or async callback); doing so returns an
/// [`crate::error::AvisoErrorKind::InvalidUsage`] error.
///
/// # Safety
///
/// `client` must be a live handle from a successful build.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_schema(client: *const AvisoClient) -> *mut AvisoOutcome {
    guard_outcome(|| {
        let Some(client) = (unsafe { client.as_ref() }) else {
            return error::invalid_input("client must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        match runtime().block_on(client.inner.schema()) {
            Ok(catalog) => match serde_json::to_string(&catalog) {
                Ok(json) => match std::ffi::CString::new(json) {
                    Ok(text) => AvisoOutcome::text(text).into_raw(),
                    Err(_) => {
                        error::internal("schema JSON contained an interior NUL").into_outcome()
                    }
                },
                Err(err) => {
                    error::internal(&format!("schema serialization failed: {err}")).into_outcome()
                }
            },
            Err(err) => error::map_error(&err).into_outcome(),
        }
    })
}
