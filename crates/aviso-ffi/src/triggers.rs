//! Trigger handles: build a per-notification side effect and attach it to a
//! watch request with `aviso_watch_request_add_trigger`.
//!
//! A trigger is created by one of the `aviso_trigger_*` factory functions,
//! tuned with the `aviso_trigger_set_*` setters, and then consumed by
//! `aviso_watch_request_add_trigger`. A setter that does not apply to a
//! trigger's kind is ignored, matching the core builder. A bad argument is
//! remembered on the handle and surfaced through the watch's `on_end` when the
//! request is started.

use std::ffi::c_char;
use std::ptr;
use std::time::Duration;

use aviso::watch::{HttpMethod, Trigger};

use crate::client::cstr_opt;
use crate::error::{self, OutcomeError};
use crate::guard;

/// HTTP method for the webhook trigger. Discriminants are fixed for ABI
/// stability and mirror the core `HttpMethod`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvisoHttpMethod {
    /// `POST` (the webhook default).
    Post = 0,
    /// `GET`.
    Get = 1,
    /// `PUT`.
    Put = 2,
    /// `PATCH`.
    Patch = 3,
    /// `DELETE`.
    Delete = 4,
}

/// Opaque trigger handle. Built by a factory, tuned by the setters, and
/// consumed by `aviso_watch_request_add_trigger` (which nulls the caller's
/// pointer) or freed with `aviso_trigger_free`.
pub struct AvisoTrigger {
    pub(crate) trigger: Option<Trigger>,
    pub(crate) error: Option<OutcomeError>,
}

impl AvisoTrigger {
    fn apply(&mut self, f: impl FnOnce(Trigger) -> Trigger) {
        if self.error.is_some() {
            return;
        }
        if let Some(trigger) = self.trigger.take() {
            self.trigger = Some(f(trigger));
        }
    }
}

fn into_handle(trigger: Trigger) -> *mut AvisoTrigger {
    Box::into_raw(Box::new(AvisoTrigger {
        trigger: Some(trigger),
        error: None,
    }))
}

fn error_handle(error: OutcomeError) -> *mut AvisoTrigger {
    Box::into_raw(Box::new(AvisoTrigger {
        trigger: None,
        error: Some(error),
    }))
}

/// Builds an echo trigger (writes each notification as compact JSON to standard
/// output). Free an abandoned trigger with `aviso_trigger_free`.
#[unsafe(no_mangle)]
pub extern "C" fn aviso_trigger_echo() -> *mut AvisoTrigger {
    guard(ptr::null_mut(), || into_handle(Trigger::echo()))
}

/// Builds a log trigger that appends each notification as compact JSON to the
/// file at `path`. A null or non-UTF-8 `path` is remembered and surfaced when
/// the watch starts.
///
/// # Safety
///
/// `path`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_log(path: *const c_char) -> *mut AvisoTrigger {
    guard(ptr::null_mut(), || match unsafe { cstr_opt(path) } {
        Some(path) => into_handle(Trigger::log(path)),
        None => error_handle(error::invalid_input(
            "log path must be non-null and valid UTF-8",
        )),
    })
}

/// Builds a command trigger that runs `/bin/sh -c <cmd>` per notification. The
/// command trigger is Unix-only: on a non-Unix target this returns a handle
/// carrying an `InvalidUsage` error (surfaced when the watch starts), so the
/// symbol exists on every platform and the ABI stays uniform. A null or
/// non-UTF-8 `cmd` is remembered and surfaced when the watch starts.
///
/// # Safety
///
/// `cmd`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_command(cmd: *const c_char) -> *mut AvisoTrigger {
    guard(ptr::null_mut(), || {
        #[cfg(unix)]
        {
            match unsafe { cstr_opt(cmd) } {
                Some(cmd) => into_handle(Trigger::command(cmd)),
                None => error_handle(error::invalid_input(
                    "command must be non-null and valid UTF-8",
                )),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = cmd;
            error_handle(error::invalid_usage("the command trigger is Unix-only"))
        }
    })
}

/// Builds a webhook trigger that sends an HTTP request per notification to
/// `url`. A null or non-UTF-8 `url` is remembered and surfaced when the watch
/// starts.
///
/// # Safety
///
/// `url`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_webhook(url: *const c_char) -> *mut AvisoTrigger {
    guard(ptr::null_mut(), || match unsafe { cstr_opt(url) } {
        Some(url) => into_handle(Trigger::webhook(url)),
        None => error_handle(error::invalid_input(
            "webhook url must be non-null and valid UTF-8",
        )),
    })
}

/// Builds a Teams trigger that posts an Adaptive Card per notification to the
/// Teams Workflows webhook `url`. A null or non-UTF-8 `url` is remembered and
/// surfaced when the watch starts.
///
/// # Safety
///
/// `url`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_teams(url: *const c_char) -> *mut AvisoTrigger {
    guard(ptr::null_mut(), || match unsafe { cstr_opt(url) } {
        Some(url) => into_handle(Trigger::teams(url)),
        None => error_handle(error::invalid_input(
            "teams url must be non-null and valid UTF-8",
        )),
    })
}

/// Builds a post trigger that forwards the raw `CloudEvent` envelope as an HTTP
/// POST per notification to `url`. A null or non-UTF-8 `url` is remembered and
/// surfaced when the watch starts.
///
/// # Safety
///
/// `url`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_post(url: *const c_char) -> *mut AvisoTrigger {
    guard(ptr::null_mut(), || match unsafe { cstr_opt(url) } {
        Some(url) => into_handle(Trigger::post(url)),
        None => error_handle(error::invalid_input(
            "post url must be non-null and valid UTF-8",
        )),
    })
}

/// Sets the echo trigger's listener-attribution label. A null or non-UTF-8
/// `label` is remembered and surfaced when the watch starts. Ignored on other
/// trigger kinds.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory. `label`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_label(trigger: *mut AvisoTrigger, label: *const c_char) {
    guard((), || {
        let Some(trigger) = (unsafe { trigger.as_mut() }) else {
            return;
        };
        if trigger.error.is_some() {
            return;
        }
        match unsafe { cstr_opt(label) } {
            Some(label) => trigger.apply(|t| t.label(label)),
            None => {
                trigger.error = Some(error::invalid_input(
                    "label must be non-null and valid UTF-8",
                ));
            }
        }
    });
}

/// Sets the retry count (additional attempts after the first failure).
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_retries(trigger: *mut AvisoTrigger, retries: u32) {
    guard((), || {
        if let Some(trigger) = unsafe { trigger.as_mut() } {
            trigger.apply(|t| t.retries(retries));
        }
    });
}

/// Sets whether the trigger is required (a required trigger's terminal failure
/// ends the watch; an optional trigger's failure is logged and the watch
/// continues).
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_required(trigger: *mut AvisoTrigger, required: bool) {
    guard((), || {
        if let Some(trigger) = unsafe { trigger.as_mut() } {
            trigger.apply(|t| t.required(required));
        }
    });
}

/// Sets a per-trigger timeout in seconds. Meaningful for the command and
/// HTTP-based triggers; ignored on echo and log.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_timeout_secs(
    trigger: *mut AvisoTrigger,
    timeout_secs: u64,
) {
    guard((), || {
        if let Some(trigger) = unsafe { trigger.as_mut() } {
            trigger.apply(|t| t.timeout(Duration::from_secs(timeout_secs)));
        }
    });
}

/// Sets the fail-fast policy on terminal failures. Meaningful for the command
/// and HTTP-based triggers.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_fail_fast(trigger: *mut AvisoTrigger, fail_fast: bool) {
    guard((), || {
        if let Some(trigger) = unsafe { trigger.as_mut() } {
            trigger.apply(|t| t.fail_fast(fail_fast));
        }
    });
}

/// Sets the HTTP method for a webhook trigger from an `AvisoHttpMethod`
/// discriminant. Taken as an integer (not the enum) so an out-of-range value
/// from C is a remembered `InvalidInput` error rather than undefined behaviour;
/// an unknown discriminant is recorded and surfaced when the watch starts.
/// Ignored on other trigger kinds.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_method(trigger: *mut AvisoTrigger, method: u32) {
    guard((), || {
        let Some(trigger) = (unsafe { trigger.as_mut() }) else {
            return;
        };
        if trigger.error.is_some() {
            return;
        }
        let method = match method {
            x if x == AvisoHttpMethod::Post as u32 => HttpMethod::Post,
            x if x == AvisoHttpMethod::Get as u32 => HttpMethod::Get,
            x if x == AvisoHttpMethod::Put as u32 => HttpMethod::Put,
            x if x == AvisoHttpMethod::Patch as u32 => HttpMethod::Patch,
            x if x == AvisoHttpMethod::Delete as u32 => HttpMethod::Delete,
            other => {
                trigger.error = Some(error::invalid_input(&format!(
                    "unknown HTTP method discriminant: {other}"
                )));
                return;
            }
        };
        trigger.apply(|t| t.method(method));
    });
}

/// Adds a request header to a webhook trigger. A null or non-UTF-8 `name` or
/// `value` is remembered and surfaced when the watch starts. Ignored on other
/// trigger kinds.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory. `name` and `value`,
/// when non-null, must be NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_header(
    trigger: *mut AvisoTrigger,
    name: *const c_char,
    value: *const c_char,
) {
    guard((), || {
        let Some(trigger) = (unsafe { trigger.as_mut() }) else {
            return;
        };
        if trigger.error.is_some() {
            return;
        }
        let (Some(name), Some(value)) = (unsafe { cstr_opt(name) }, unsafe { cstr_opt(value) })
        else {
            trigger.error = Some(error::invalid_input(
                "header name and value must be non-null and valid UTF-8",
            ));
            return;
        };
        trigger.apply(|t| t.header(name, value));
    });
}

/// Sets the body template for a webhook trigger. A null or non-UTF-8 `body` is
/// remembered and surfaced when the watch starts. Ignored on other trigger
/// kinds.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory. `body`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_body_template(
    trigger: *mut AvisoTrigger,
    body: *const c_char,
) {
    guard((), || {
        let Some(trigger) = (unsafe { trigger.as_mut() }) else {
            return;
        };
        if trigger.error.is_some() {
            return;
        }
        match unsafe { cstr_opt(body) } {
            Some(body) => trigger.apply(|t| t.body_template(body)),
            None => {
                trigger.error = Some(error::invalid_input(
                    "body template must be non-null and valid UTF-8",
                ));
            }
        }
    });
}

/// Adds an environment variable to a command trigger's child process. The
/// command trigger is Unix-only; on a non-Unix target this records an
/// `InvalidUsage` error. A null or non-UTF-8 `key` or `value` is remembered and
/// surfaced when the watch starts. Ignored on other trigger kinds.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory. `key` and `value`,
/// when non-null, must be NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_env(
    trigger: *mut AvisoTrigger,
    key: *const c_char,
    value: *const c_char,
) {
    guard((), || {
        let Some(trigger) = (unsafe { trigger.as_mut() }) else {
            return;
        };
        if trigger.error.is_some() {
            return;
        }
        #[cfg(unix)]
        {
            let (Some(key), Some(value)) = (unsafe { cstr_opt(key) }, unsafe { cstr_opt(value) })
            else {
                trigger.error = Some(error::invalid_input(
                    "env key and value must be non-null and valid UTF-8",
                ));
                return;
            };
            trigger.apply(|t| t.env(key, value));
        }
        #[cfg(not(unix))]
        {
            let _ = (key, value);
            trigger.error = Some(error::invalid_usage("the command trigger is Unix-only"));
        }
    });
}

/// Sets the working directory for a command trigger's child process. The
/// command trigger is Unix-only; on a non-Unix target this records an
/// `InvalidUsage` error. A null or non-UTF-8 `dir` is remembered and surfaced
/// when the watch starts. Ignored on other trigger kinds.
///
/// # Safety
///
/// `trigger` must be a live handle from a trigger factory. `dir`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_set_working_dir(
    trigger: *mut AvisoTrigger,
    dir: *const c_char,
) {
    guard((), || {
        let Some(trigger) = (unsafe { trigger.as_mut() }) else {
            return;
        };
        if trigger.error.is_some() {
            return;
        }
        #[cfg(unix)]
        {
            match unsafe { cstr_opt(dir) } {
                Some(dir) => trigger.apply(|t| t.working_dir(dir)),
                None => {
                    trigger.error = Some(error::invalid_input(
                        "working dir must be non-null and valid UTF-8",
                    ));
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = dir;
            trigger.error = Some(error::invalid_usage("the command trigger is Unix-only"));
        }
    });
}

/// Frees an abandoned trigger. Triggers consumed by
/// `aviso_watch_request_add_trigger` are already freed; calling this on the
/// nulled-out pointer is a safe no-op.
///
/// # Safety
///
/// `trigger`, when non-null, must be a live handle from a trigger factory that
/// was not consumed by an add.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_trigger_free(trigger: *mut AvisoTrigger) {
    if trigger.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(trigger) });
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn echo_factory_builds_and_frees() {
        let trigger = aviso_trigger_echo();
        assert!(!trigger.is_null());
        unsafe { aviso_trigger_free(trigger) };
    }

    #[test]
    fn log_with_null_path_remembers_an_error() {
        let trigger = unsafe { aviso_trigger_log(ptr::null()) };
        assert!(!trigger.is_null());
        assert!(unsafe { (*trigger).error.is_some() });
        assert!(unsafe { (*trigger).trigger.is_none() });
        unsafe { aviso_trigger_free(trigger) };
    }

    #[test]
    fn setters_on_a_good_trigger_keep_it_valid() {
        let url = CString::new("https://example.invalid/hook").expect("cstring");
        let trigger = unsafe { aviso_trigger_webhook(url.as_ptr()) };
        unsafe { aviso_trigger_set_retries(trigger, 3) };
        unsafe { aviso_trigger_set_method(trigger, AvisoHttpMethod::Put as u32) };
        unsafe { aviso_trigger_set_timeout_secs(trigger, 5) };
        assert!(unsafe { (*trigger).error.is_none() });
        assert!(unsafe { (*trigger).trigger.is_some() });
        unsafe { aviso_trigger_free(trigger) };
    }

    #[test]
    fn set_method_with_unknown_discriminant_remembers_invalid_input() {
        let url = CString::new("https://example.invalid/hook").expect("cstring");
        let trigger = unsafe { aviso_trigger_webhook(url.as_ptr()) };
        unsafe { aviso_trigger_set_method(trigger, 99) };
        assert!(unsafe { (*trigger).error.is_some() });
        unsafe { aviso_trigger_free(trigger) };
    }

    #[test]
    fn first_remembered_error_wins() {
        // A second bad argument must not overwrite the first remembered error.
        let trigger = unsafe { aviso_trigger_log(ptr::null()) };
        unsafe { aviso_trigger_set_body_template(trigger, ptr::null()) };
        unsafe { aviso_trigger_set_method(trigger, 99) };
        assert!(unsafe { (*trigger).error.is_some() });
        unsafe { aviso_trigger_free(trigger) };
    }
}
