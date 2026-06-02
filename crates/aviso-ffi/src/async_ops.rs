//! Completion-callback async variants of the blocking verbs.
//!
//! Each `aviso_client_*_async` function returns immediately after spawning a
//! task on the process-global runtime; when the verb finishes, `on_complete`
//! is called exactly once with an owning `AvisoOutcome` (the receiver frees
//! it). It normally runs on a runtime thread, and on the calling thread only in
//! the rare case where the runtime cannot be started (so completion never
//! unwinds across the boundary). The callback itself must not unwind; if it
//! does, that outcome may be leaked rather than risk a double free. Unlike the
//! blocking verbs these never `block_on`, so they are safe to call from a watch
//! or async callback. The facade builds a `std::future` on top of this surface.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use aviso::NotificationRequest;
use serde_json::Value;

use crate::client::{AvisoClient, cstr_nullable, cstr_opt, parse_identifier};
use crate::error::{self, OutcomeError};
use crate::outcome::AvisoOutcome;
use crate::runtime;
use crate::send::{SendOutcome, SendPtr};

/// C callback invoked once when an async verb completes. It takes ownership of
/// `outcome` and must free it with `aviso_outcome_free`.
type OnComplete = extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome);

/// Hands `outcome` to `on_complete`, trapping any Rust unwind so it never
/// crosses back into the runtime. Called from the spawned task. Callbacks must
/// not unwind; if one does, the outcome it was handed is leaked rather than
/// freed here, since whether the callback already freed it is unknown and a
/// double free would be worse.
fn complete(on_complete: OnComplete, ctx: SendPtr, outcome: AvisoOutcome) {
    let raw = outcome.into_raw();
    let _ = catch_unwind(AssertUnwindSafe(|| on_complete(ctx.0, raw)));
}

/// Runs the synchronous prelude of an async verb, trapping a panic and
/// reporting it through `on_complete` as a Panic outcome. A panic can only
/// happen before the single `spawn` (the prelude does nothing afterwards), so
/// `on_complete` still fires exactly once: either from the spawned task or,
/// on a trapped panic, from here.
fn guard_async(on_complete: OnComplete, ctx: *mut c_void, body: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(body)).is_err() {
        deliver_error(on_complete, ctx, error::panic_error());
    }
}

/// Reports a pre-spawn argument error through `on_complete`, on a runtime
/// thread, so the delivery path is uniform with the success case. The error
/// outcome is built now and moved into the task as a `Send` pointer.
fn deliver_error(on_complete: OnComplete, ctx: *mut c_void, error: OutcomeError) {
    let send_ctx = SendPtr(ctx);
    let raw = AvisoOutcome::error(error).into_raw();
    let send_outcome = SendOutcome(raw);
    let spawned = catch_unwind(AssertUnwindSafe(move || {
        runtime().spawn(async move {
            // Rebind the whole newtypes so the future captures the Send
            // wrappers, not their inner raw-pointer fields (disjoint capture).
            let send_ctx = send_ctx;
            let send_outcome = send_outcome;
            let raw = send_outcome.0;
            let _ = catch_unwind(AssertUnwindSafe(|| on_complete(send_ctx.0, raw)));
        });
    }));
    if spawned.is_err() {
        // The runtime could not be obtained (a fatal init failure trapped here
        // rather than unwound across the boundary). Complete synchronously so
        // on_complete still fires exactly once; `raw` is a copy of the
        // still-live outcome pointer, and the spawn path never ran the callback.
        let _ = catch_unwind(AssertUnwindSafe(|| on_complete(ctx, raw)));
    }
}

/// Publishes a notification asynchronously. When it finishes, `on_complete` is
/// called exactly once (normally on a runtime thread) with an owning outcome
/// (the response JSON, or a structured error): free it with
/// `aviso_outcome_free`. The callback must not unwind across the boundary.
/// No-op if `on_complete` is null.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `event_type`,
/// `identifier_json`, and `payload_json`, when non-null, must be NUL-terminated
/// C strings. `ctx` is passed verbatim to `on_complete`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_notify_async(
    client: *const AvisoClient,
    event_type: *const c_char,
    identifier_json: *const c_char,
    payload_json: *const c_char,
    on_complete: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) {
    let Some(on_complete) = on_complete else {
        return;
    };
    guard_async(on_complete, ctx, || {
        let Some(client) = (unsafe { client.as_ref() }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("client must not be null"),
            );
            return;
        };
        let Some(event_type) = (unsafe { cstr_opt(event_type) }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("event_type must be non-null and valid UTF-8"),
            );
            return;
        };
        let identifier = match unsafe { cstr_nullable(identifier_json) } {
            Ok(None) => BTreeMap::new(),
            Ok(Some(text)) => match parse_identifier(text) {
                Ok(identifier) => identifier,
                Err(err) => {
                    deliver_error(on_complete, ctx, err);
                    return;
                }
            },
            Err(()) => {
                deliver_error(
                    on_complete,
                    ctx,
                    error::invalid_input("identifier_json must be valid UTF-8"),
                );
                return;
            }
        };
        let payload = match unsafe { cstr_nullable(payload_json) } {
            Ok(None) => None,
            Ok(Some(text)) => match serde_json::from_str::<Value>(text) {
                Ok(value) => Some(value),
                Err(err) => {
                    deliver_error(
                        on_complete,
                        ctx,
                        error::invalid_input(&format!("payload_json is not valid JSON: {err}")),
                    );
                    return;
                }
            },
            Err(()) => {
                deliver_error(
                    on_complete,
                    ctx,
                    error::invalid_input("payload_json must be valid UTF-8"),
                );
                return;
            }
        };

        let mut request = NotificationRequest::new(event_type).with_identifier(identifier);
        if let Some(payload) = payload {
            request = request.with_payload(payload);
        }

        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        runtime().spawn(async move {
            let send_ctx = send_ctx;
            let outcome = match client.notify(&request).await {
                Ok(response) => AvisoOutcome::json_text(serde_json::to_string(&response)),
                Err(err) => AvisoOutcome::error(error::map_error(&err)),
            };
            complete(on_complete, send_ctx, outcome);
        });
    });
}

/// Fetches the full schema catalog asynchronously. When it finishes,
/// `on_complete` is called exactly once (normally on a runtime thread) with an
/// owning outcome (the catalog JSON, or a structured error): free it with
/// `aviso_outcome_free`. The callback must not unwind across the boundary.
/// No-op if `on_complete` is null.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `ctx` is passed
/// verbatim to `on_complete`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_schema_async(
    client: *const AvisoClient,
    on_complete: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) {
    let Some(on_complete) = on_complete else {
        return;
    };
    guard_async(on_complete, ctx, || {
        let Some(client) = (unsafe { client.as_ref() }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("client must not be null"),
            );
            return;
        };
        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        runtime().spawn(async move {
            let send_ctx = send_ctx;
            let outcome = match client.schema().await {
                Ok(catalog) => AvisoOutcome::json_text(serde_json::to_string(&catalog)),
                Err(err) => AvisoOutcome::error(error::map_error(&err)),
            };
            complete(on_complete, send_ctx, outcome);
        });
    });
}

/// Fetches one stream's schema asynchronously. When it finishes, `on_complete`
/// is called exactly once (normally on a runtime thread) with an owning outcome (the
/// schema JSON, or a structured error): free it with `aviso_outcome_free`. The
/// callback must not unwind across the boundary. No-op if `on_complete` is null.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `event_type`, when
/// non-null, must be a NUL-terminated C string. `ctx` is passed verbatim to
/// `on_complete`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_schema_for_async(
    client: *const AvisoClient,
    event_type: *const c_char,
    on_complete: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) {
    let Some(on_complete) = on_complete else {
        return;
    };
    guard_async(on_complete, ctx, || {
        let Some(client) = (unsafe { client.as_ref() }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("client must not be null"),
            );
            return;
        };
        let Some(event_type) = (unsafe { cstr_opt(event_type) }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("event_type must be non-null and valid UTF-8"),
            );
            return;
        };
        let event_type = event_type.to_string();
        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        runtime().spawn(async move {
            let send_ctx = send_ctx;
            let outcome = match client.schema_for(&event_type).await {
                Ok(response) => AvisoOutcome::json_text(serde_json::to_string(&response)),
                Err(err) => AvisoOutcome::error(error::map_error(&err)),
            };
            complete(on_complete, send_ctx, outcome);
        });
    });
}

/// Wipes one stream asynchronously (operator-only). When it finishes,
/// `on_complete` is called exactly once (normally on a runtime thread) with an
/// owning outcome (an empty success, or a structured error): free it with
/// `aviso_outcome_free`. The callback must not unwind across the boundary.
/// No-op if `on_complete` is null.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `stream_name`, when
/// non-null, must be a NUL-terminated C string. `ctx` is passed verbatim to
/// `on_complete`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_wipe_stream_async(
    client: *const AvisoClient,
    stream_name: *const c_char,
    on_complete: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) {
    let Some(on_complete) = on_complete else {
        return;
    };
    guard_async(on_complete, ctx, || {
        let Some(client) = (unsafe { client.as_ref() }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("client must not be null"),
            );
            return;
        };
        let Some(stream_name) = (unsafe { cstr_opt(stream_name) }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("stream_name must be non-null and valid UTF-8"),
            );
            return;
        };
        let stream_name = stream_name.to_string();
        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        runtime().spawn(async move {
            let send_ctx = send_ctx;
            let outcome = match client.wipe_stream(&stream_name).await {
                Ok(()) => AvisoOutcome::empty(),
                Err(err) => AvisoOutcome::error(error::map_error(&err)),
            };
            complete(on_complete, send_ctx, outcome);
        });
    });
}

/// Wipes every stream asynchronously (operator-only). When it finishes,
/// `on_complete` is called exactly once (normally on a runtime thread) with an
/// owning outcome (an empty success, or a structured error): free it with
/// `aviso_outcome_free`. The callback must not unwind across the boundary.
/// No-op if `on_complete` is null.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `ctx` is passed
/// verbatim to `on_complete`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_wipe_all_async(
    client: *const AvisoClient,
    on_complete: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) {
    let Some(on_complete) = on_complete else {
        return;
    };
    guard_async(on_complete, ctx, || {
        let Some(client) = (unsafe { client.as_ref() }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("client must not be null"),
            );
            return;
        };
        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        runtime().spawn(async move {
            let send_ctx = send_ctx;
            let outcome = match client.wipe_all().await {
                Ok(()) => AvisoOutcome::empty(),
                Err(err) => AvisoOutcome::error(error::map_error(&err)),
            };
            complete(on_complete, send_ctx, outcome);
        });
    });
}

/// Deletes one notification by its `<event_type>@<sequence>` id asynchronously
/// (operator-only). When it finishes, `on_complete` is called exactly once
/// (normally on a runtime thread) with an owning outcome (an empty success, or
/// a structured error): free it with `aviso_outcome_free`. The callback must
/// not unwind across the boundary. No-op if `on_complete` is null.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `notification_id`,
/// when non-null, must be a NUL-terminated C string. `ctx` is passed verbatim
/// to `on_complete`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_delete_notification_async(
    client: *const AvisoClient,
    notification_id: *const c_char,
    on_complete: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) {
    let Some(on_complete) = on_complete else {
        return;
    };
    guard_async(on_complete, ctx, || {
        let Some(client) = (unsafe { client.as_ref() }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("client must not be null"),
            );
            return;
        };
        let Some(notification_id) = (unsafe { cstr_opt(notification_id) }) else {
            deliver_error(
                on_complete,
                ctx,
                error::invalid_input("notification_id must be non-null and valid UTF-8"),
            );
            return;
        };
        let notification_id = notification_id.to_string();
        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        runtime().spawn(async move {
            let send_ctx = send_ctx;
            let outcome = match client.delete_notification(&notification_id).await {
                Ok(()) => AvisoOutcome::empty(),
                Err(err) => AvisoOutcome::error(error::map_error(&err)),
            };
            complete(on_complete, send_ctx, outcome);
        });
    });
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
mod tests {
    use super::*;
    use crate::AvisoErrorKind;
    use crate::client::{aviso_client_builder_build, aviso_client_builder_new, aviso_client_free};
    use crate::outcome::{aviso_outcome_error, aviso_outcome_free, aviso_outcome_take_client};
    use std::ffi::CString;
    use std::ptr;
    use std::sync::mpsc;
    use std::time::Duration;

    struct Sink {
        kind: mpsc::Sender<i32>,
    }

    extern "C" fn on_complete(ctx: *mut c_void, outcome: *mut AvisoOutcome) {
        let sink = unsafe { &*(ctx as *const Sink) };
        let error = unsafe { aviso_outcome_error(outcome) };
        let kind = if error.is_null() {
            -1
        } else {
            unsafe { (*error).kind as i32 }
        };
        let _ = sink.kind.send(kind);
        unsafe { aviso_outcome_free(outcome) };
    }

    fn build_client() -> *mut AvisoClient {
        let base = CString::new("http://127.0.0.1:1").expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(base.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        unsafe { aviso_outcome_free(outcome) };
        assert!(!client.is_null());
        client
    }

    #[test]
    fn notify_async_with_bad_identifier_reports_invalid_input_via_callback() {
        // The argument error is delivered through on_complete on a runtime
        // thread, without any network work.
        let client = build_client();
        let (tx, rx) = mpsc::channel();
        // Leak the sink so a panic here (e.g. a timeout) cannot drop it while
        // the spawned task may still call on_complete with this ctx.
        let sink: &'static Sink = Box::leak(Box::new(Sink { kind: tx }));
        let ctx = std::ptr::from_ref(sink) as *mut c_void;

        let event = CString::new("test_event").expect("cstring");
        let bad = CString::new("{not json").expect("cstring");
        unsafe {
            aviso_client_notify_async(
                client,
                event.as_ptr(),
                bad.as_ptr(),
                ptr::null(),
                Some(on_complete),
                ctx,
            );
        }

        let kind = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("on_complete fired");
        assert_eq!(kind, AvisoErrorKind::InvalidInput as i32);
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn schema_async_with_null_callback_is_a_noop() {
        let client = build_client();
        unsafe { aviso_client_schema_async(client, None, ptr::null_mut()) };
        unsafe { aviso_client_free(client) };
    }
}
