// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The watch surface: a request-builder handle, per-notification views, and a
//! callback-driven watch handle with an explicit stop/wait/free lifecycle.
//!
//! A watch runs on the process-global runtime. `aviso_client_watch` spawns a
//! task that opens the stream and loops over it, calling `on_notification` per
//! item and `on_end` exactly once when the stream ends or fails. The task is
//! driven by the synchronous `watch()` plus `recv()`, not the core's
//! handler-shaped `watch_with_handler`, so the C `on_notification` can request
//! a graceful stop by returning `false` (per ADR D21).

use std::collections::BTreeMap;
use std::ffi::{CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aviso::watch::{ResumeStart, Trigger, WatchRequest};
use aviso::{ClientError, Notification};
use serde_json::Value;
use tokio::sync::Notify;

use crate::client::{AvisoClient, cstr_nullable, cstr_opt};
use crate::error::{self, OutcomeError};
use crate::outcome::AvisoOutcome;
use crate::send::{SendOutcome, SendPtr};
use crate::triggers::AvisoTrigger;
use crate::{guard, guard_outcome, reject_blocking_on_runtime, runtime};

/// C callback invoked once per notification. Returning `false` requests a
/// graceful stop; the borrowed `notification` is valid only for the call.
type OnNotification =
    extern "C" fn(ctx: *mut c_void, notification: *const AvisoNotification) -> bool;

/// C callback invoked exactly once when the watch ends or fails. It takes
/// ownership of `outcome` and must free it with `aviso_outcome_free`.
type OnEnd = extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome);

/// Opaque builder for a watch request. Setters mutate it in place; the first
/// bad argument is remembered and surfaced through `on_end` when the watch
/// starts. Consumed by `aviso_client_watch` (which nulls the caller's pointer)
/// or freed with `aviso_watch_request_free`.
pub struct AvisoWatchRequest {
    spec: Option<RequestSpec>,
    error: Option<OutcomeError>,
}

struct RequestSpec {
    event_type: String,
    filter: Option<BTreeMap<String, Value>>,
    mode: Mode,
    triggers: Vec<Trigger>,
}

enum Mode {
    Watch,
    WatchFrom(ResumeStart),
    ReplayOnly(ResumeStart),
}

impl RequestSpec {
    fn into_request(self) -> WatchRequest {
        let mut request = match self.mode {
            Mode::Watch => WatchRequest::watch(self.event_type),
            Mode::WatchFrom(start) => WatchRequest::watch_from(self.event_type, start),
            Mode::ReplayOnly(start) => WatchRequest::replay_only(self.event_type, start),
        };
        if let Some(filter) = self.filter {
            request = request.with_filter(filter);
        }
        if !self.triggers.is_empty() {
            request = request.with_triggers(self.triggers);
        }
        request
    }
}

impl AvisoWatchRequest {
    fn with_spec(&mut self, f: impl FnOnce(&mut RequestSpec)) {
        if self.error.is_some() {
            return;
        }
        if let Some(spec) = self.spec.as_mut() {
            f(spec);
        }
    }
}

/// Parses the `filter_json` argument: a JSON object whose values are arbitrary
/// JSON (a scalar for an exact match, an object for a spatial or range rule),
/// matching the core filter shape `BTreeMap<String, serde_json::Value>`.
fn parse_filter(text: &str) -> Result<BTreeMap<String, Value>, OutcomeError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|err| error::invalid_input(&format!("filter_json is not valid JSON: {err}")))?;
    match value {
        Value::Object(object) => Ok(object.into_iter().collect()),
        _ => Err(error::invalid_input("filter_json must be a JSON object")),
    }
}

/// Creates a live-watch request for `event_type`. Returns a handle (null only
/// if an internal panic is trapped); a null or non-UTF-8 `event_type` is
/// remembered and surfaced when the watch starts. Free an abandoned request
/// with `aviso_watch_request_free`.
///
/// # Safety
///
/// `event_type`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_new(
    event_type: *const c_char,
) -> *mut AvisoWatchRequest {
    guard(ptr::null_mut(), || {
        let mut request = AvisoWatchRequest {
            spec: None,
            error: None,
        };
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(event_type) } {
            Some(event_type) => {
                request.spec = Some(RequestSpec {
                    event_type: event_type.to_string(),
                    filter: None,
                    mode: Mode::Watch,
                    triggers: Vec::new(),
                });
            }
            None => {
                request.error = Some(error::invalid_input(
                    "event_type must be non-null and valid UTF-8",
                ));
            }
        }
        Box::into_raw(Box::new(request))
    })
}

/// Sets the identifier filter from a JSON object string (a null argument clears
/// it). A non-UTF-8 or non-object argument is remembered and surfaced when the
/// watch starts.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`.
/// `filter_json`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_set_filter_json(
    request: *mut AvisoWatchRequest,
    filter_json: *const c_char,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if request.error.is_some() {
            return;
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_nullable(filter_json) } {
            Ok(None) => request.with_spec(|spec| spec.filter = None),
            Ok(Some(text)) => match parse_filter(text) {
                Ok(filter) => request.with_spec(|spec| spec.filter = Some(filter)),
                Err(err) => request.error = Some(err),
            },
            Err(()) => {
                request.error = Some(error::invalid_input("filter_json must be valid UTF-8"));
            }
        }
    });
}

/// Resumes a live watch after the given sequence (the watch replays everything
/// after `sequence`, then goes live).
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_watch_from_sequence(
    request: *mut AvisoWatchRequest,
    sequence: u64,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        if let Some(request) = unsafe { request.as_mut() } {
            request.with_spec(|spec| {
                spec.mode = Mode::WatchFrom(ResumeStart::AfterSequence(sequence));
            });
        }
    });
}

/// Resumes a live watch from the given date string (server-defined format),
/// replaying from that point and then going live.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`. `date`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_watch_from_date(
    request: *mut AvisoWatchRequest,
    date: *const c_char,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if request.error.is_some() {
            return;
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(date) } {
            Some(date) => request
                .with_spec(|spec| spec.mode = Mode::WatchFrom(ResumeStart::Date(date.to_string()))),
            None => {
                request.error = Some(error::invalid_input(
                    "date must be non-null and valid UTF-8",
                ));
            }
        }
    });
}

/// Switches the request to replay-only from the given sequence: it replays
/// everything after `sequence` and then ends, without going live.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_replay_from_sequence(
    request: *mut AvisoWatchRequest,
    sequence: u64,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        if let Some(request) = unsafe { request.as_mut() } {
            request.with_spec(|spec| {
                spec.mode = Mode::ReplayOnly(ResumeStart::AfterSequence(sequence));
            });
        }
    });
}

/// Switches the request to replay-only from the given date string: it replays
/// from that point and then ends, without going live.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`. `date`, when
/// non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_replay_from_date(
    request: *mut AvisoWatchRequest,
    date: *const c_char,
) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if request.error.is_some() {
            return;
        }
        // SAFETY: each string argument is null or a NUL-terminated C string
        // that stays valid for this call, per this function's # Safety.
        match unsafe { cstr_opt(date) } {
            Some(date) => request.with_spec(|spec| {
                spec.mode = Mode::ReplayOnly(ResumeStart::Date(date.to_string()));
            }),
            None => {
                request.error = Some(error::invalid_input(
                    "date must be non-null and valid UTF-8",
                ));
            }
        }
    });
}

/// Attaches a trigger to the request, consuming the trigger handle: on entry
/// the trigger is taken and the caller's pointer is nulled (so a later free is
/// a safe no-op). A trigger built from a bad argument is remembered on the
/// request and surfaced through the watch's `on_end` when it starts.
///
/// # Safety
///
/// `request` must be a live handle from `aviso_watch_request_new`. `trigger`
/// must point to a trigger-handle pointer; `*trigger`, when non-null, must be a
/// live handle from a trigger factory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_add_trigger(
    request: *mut AvisoWatchRequest,
    trigger: *mut *mut AvisoTrigger,
) {
    guard((), || {
        if trigger.is_null() {
            return;
        }
        // SAFETY: `trigger` is non-null (checked above) and, per this
        // function's # Safety, points to a valid, aligned handle slot that
        // nothing else touches during this call.
        let slot = unsafe { &mut *trigger };
        if slot.is_null() {
            return;
        }
        // Take ownership and null the caller's pointer before any other work.
        // SAFETY: the caller's slot holds a handle this library created and has
        // not freed, and ownership passes back here once; the slot is nulled
        // right after so a second call is a no-op, per this function's #
        // Safety.
        let owned = unsafe { Box::from_raw(*slot) };
        *slot = ptr::null_mut();

        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(request) = (unsafe { request.as_mut() }) else {
            return;
        };
        if let Some(err) = owned.error {
            if request.error.is_none() {
                request.error = Some(err);
            }
            return;
        }
        if let Some(trigger) = owned.trigger {
            request.with_spec(|spec| spec.triggers.push(trigger));
        }
    });
}

/// Frees an abandoned watch request. Requests consumed by `aviso_client_watch`
/// are already freed; calling this on the nulled-out pointer is a safe no-op.
///
/// # Safety
///
/// `request`, when non-null, must be a live handle from
/// `aviso_watch_request_new` that was not consumed by a watch start.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_request_free(request: *mut AvisoWatchRequest) {
    if request.is_null() {
        return;
    }
    // SAFETY: the pointer came from this library's constructor and is freed at
    // most once, per this function's # Safety.
    drop(unsafe { Box::from_raw(request) });
}

/// A read-only view of one notification, valid only for the `on_notification`
/// call it is passed to. The `identifier` and `payload` are pre-serialized to
/// compact JSON; the accessors return borrowed pointers into this view.
pub struct AvisoNotification {
    event_type: CString,
    identifier_json: CString,
    payload_json: CString,
    sequence: u64,
}

impl AvisoNotification {
    fn from_core(notification: &Notification) -> Self {
        let identifier_json =
            serde_json::to_string(&notification.identifier).unwrap_or_else(|_| "{}".to_string());
        let payload_json =
            serde_json::to_string(&notification.payload).unwrap_or_else(|_| "null".to_string());
        Self {
            event_type: error::cstring_lossy(notification.event_type.clone()),
            identifier_json: error::cstring_lossy(identifier_json),
            payload_json: error::cstring_lossy(payload_json),
            sequence: notification.sequence,
        }
    }
}

/// Returns the notification's event type. The pointer is valid only for the
/// duration of the `on_notification` call.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_event_type(
    notification: *const AvisoNotification,
) -> *const c_char {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.event_type.as_ptr(),
        None => ptr::null(),
    }
}

/// Returns the notification's per-stream sequence number.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call. A null pointer returns `0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_sequence(
    notification: *const AvisoNotification,
) -> u64 {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.sequence,
        None => 0,
    }
}

/// Returns the notification's identifier as a compact-JSON object string. The
/// pointer is valid only for the duration of the `on_notification` call.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_identifier_json(
    notification: *const AvisoNotification,
) -> *const c_char {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.identifier_json.as_ptr(),
        None => ptr::null(),
    }
}

/// Returns the notification's payload as a compact-JSON string (`null` when the
/// payload was absent). The pointer is valid only for the duration of the
/// `on_notification` call.
///
/// # Safety
///
/// `notification` must be the pointer passed to the current `on_notification`
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_notification_payload_json(
    notification: *const AvisoNotification,
) -> *const c_char {
    // SAFETY: the handle is null or one this library handed out and the caller
    // has not freed, per this function's # Safety; as_ref/as_mut return None
    // for null.
    match unsafe { notification.as_ref() } {
        Some(notification) => notification.payload_json.as_ptr(),
        None => ptr::null(),
    }
}

/// Cooperative stop signal shared by the watch handle and its task.
struct StopSignal {
    flag: AtomicBool,
    notify: Notify,
}

/// Opaque handle to a running watch. Stop it with `aviso_watch_stop`, block for
/// its completion with `aviso_watch_wait`, and release it with
/// `aviso_watch_free`.
pub struct AvisoWatch {
    stop: Arc<StopSignal>,
    join: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Why a watch loop ended. Holds the core `ClientError` (which is `Send`)
/// rather than a built `OutcomeError` (which is not), so it can be carried
/// across the `close().await` and mapped to an outcome afterwards.
enum EndReason {
    Graceful,
    Failed(ClientError),
    Panicked,
}

/// Drives one watch to completion, returning the outcome to hand to `on_end`.
async fn run_watch(
    client: aviso::AvisoClient,
    request: WatchRequest,
    on_notification: OnNotification,
    ctx: SendPtr,
    stop: &StopSignal,
) -> AvisoOutcome {
    let mut stream = match client.watch(request) {
        Ok(stream) => stream,
        Err(err) => return AvisoOutcome::error(error::map_error(&err)),
    };

    let reason = loop {
        if stop.flag.load(Ordering::Acquire) {
            break EndReason::Graceful;
        }
        tokio::select! {
            biased;
            () = stop.notify.notified() => break EndReason::Graceful,
            item = stream.recv() => match item {
                Some(Ok(notification)) => {
                    let view = AvisoNotification::from_core(&notification);
                    let view_ptr = &raw const view;
                    match catch_unwind(AssertUnwindSafe(|| on_notification(ctx.0, view_ptr))) {
                        Ok(true) => {}
                        Ok(false) => break EndReason::Graceful,
                        Err(_) => break EndReason::Panicked,
                    }
                }
                Some(Err(err)) => break EndReason::Failed(err),
                None => break EndReason::Graceful,
            },
        }
    };

    // Close (rather than drop) so the supervisor's final cursor flush lands
    // before the task returns; close also runs the D19 drop-to-cancel path.
    stream.close().await;

    match reason {
        EndReason::Graceful => AvisoOutcome::empty(),
        EndReason::Failed(err) => AvisoOutcome::error(error::map_error(&err)),
        EndReason::Panicked => AvisoOutcome::error(error::panic_error()),
    }
}

/// Hands `outcome` to `on_end`, trapping any Rust unwind so it never crosses
/// back into the runtime. `on_end` takes ownership and the C side frees it.
fn deliver_end(on_end: OnEnd, ctx: SendPtr, outcome: AvisoOutcome) {
    let raw = outcome.into_raw();
    let _ = catch_unwind(AssertUnwindSafe(|| on_end(ctx.0, raw)));
}

/// Starts a watch, consuming the request. The watch runs on the global runtime
/// and returns at once; `on_notification` fires per notification (returning
/// `false` requests a graceful stop) and `on_end` fires exactly once when the
/// stream ends or fails, taking ownership of the outcome. Every watch error,
/// at start or mid-stream, arrives through `on_end`.
///
/// Returns null when `client`, the request, `on_notification`, or `on_end` is
/// null (no watch starts), or if an internal panic is trapped. On entry the
/// request is taken and the caller's pointer is nulled, so a later free is a
/// safe no-op.
///
/// The callbacks run on a watch (runtime) thread, so they must be thread-safe,
/// must not unwind across the boundary, and must not make blocking aviso calls.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `request` must point
/// to a request-handle pointer; `*request`, when non-null, must be a live
/// handle from `aviso_watch_request_new`. `ctx` is passed verbatim to the
/// callbacks and must stay valid until `on_end` has returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_watch(
    client: *const AvisoClient,
    request: *mut *mut AvisoWatchRequest,
    on_notification: Option<
        extern "C" fn(ctx: *mut c_void, notification: *const AvisoNotification) -> bool,
    >,
    on_end: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) -> *mut AvisoWatch {
    guard(ptr::null_mut(), || {
        if request.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: `request` is non-null (checked above) and, per this
        // function's # Safety, points to a valid, aligned handle slot that
        // nothing else touches during this call.
        let slot = unsafe { &mut *request };
        if slot.is_null() {
            return ptr::null_mut();
        }
        // Take ownership and null the caller's pointer before any validation,
        // so a later free is a safe no-op even when the start is rejected.
        // SAFETY: the caller's slot holds a handle this library created and has
        // not freed, and ownership passes back here once; the slot is nulled
        // right after so a second call is a no-op, per this function's #
        // Safety.
        let owned = unsafe { Box::from_raw(*slot) };
        *slot = ptr::null_mut();

        let (Some(on_notification), Some(on_end)) = (on_notification, on_end) else {
            return ptr::null_mut();
        };
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return ptr::null_mut();
        };

        // Build the start error into an owned outcome now (a Send pointer), so
        // the task captures only Send values: the raw OutcomeError holds the
        // AvisoError view's borrowed pointers and is not Send.
        let built: Result<WatchRequest, SendOutcome> = match (owned.error, owned.spec) {
            (Some(err), _) => Err(SendOutcome(AvisoOutcome::error(err).into_raw())),
            (None, Some(spec)) => Ok(spec.into_request()),
            (None, None) => Err(SendOutcome(
                AvisoOutcome::error(error::internal("watch request was already consumed"))
                    .into_raw(),
            )),
        };

        let client = client.inner.clone();
        let stop = Arc::new(StopSignal {
            flag: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let task_stop = Arc::clone(&stop);
        let send_ctx = SendPtr(ctx);

        let join = runtime().spawn(async move {
            let send_ctx = send_ctx;
            let request = match built {
                Ok(request) => request,
                Err(send_outcome) => {
                    let raw = send_outcome.0;
                    let _ = catch_unwind(AssertUnwindSafe(|| on_end(send_ctx.0, raw)));
                    return;
                }
            };
            let outcome = run_watch(client, request, on_notification, send_ctx, &task_stop).await;
            deliver_end(on_end, send_ctx, outcome);
        });

        Box::into_raw(Box::new(AvisoWatch {
            stop,
            join: std::sync::Mutex::new(Some(join)),
        }))
    })
}

/// Requests a graceful stop. Nonblocking, idempotent, and safe to call from a
/// callback. A callback already running finishes; no new callback starts after
/// the stop is observed. `on_end` still fires once. A null pointer is a no-op.
///
/// # Safety
///
/// `watch`, when non-null, must be a live handle from `aviso_client_watch`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_stop(watch: *const AvisoWatch) {
    guard((), || {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        if let Some(watch) = unsafe { watch.as_ref() } {
            watch.stop.flag.store(true, Ordering::Release);
            watch.stop.notify.notify_one();
        }
    });
}

/// Blocks until the watch has fully ended and its `on_end` has returned.
/// Returns an empty success outcome, or an `AvisoErrorKind_InvalidUsage` error
/// when called from inside a callback (a runtime thread). Idempotent: a second
/// call after completion returns success at once.
///
/// # Safety
///
/// `watch` must be a live handle from `aviso_client_watch`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_wait(watch: *const AvisoWatch) -> *mut AvisoOutcome {
    guard_outcome(|| {
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref/as_mut
        // return None for null.
        let Some(watch) = (unsafe { watch.as_ref() }) else {
            return error::invalid_input("watch must not be null").into_outcome();
        };
        if let Some(err) = reject_blocking_on_runtime() {
            return err.into_outcome();
        }
        // A poisoned lock means a prior panic held it; surface that rather than
        // masking it as a completed watch.
        let Ok(mut guard) = watch.join.lock() else {
            return error::internal("the watch handle lock was poisoned by a prior panic")
                .into_outcome();
        };
        // Await through the stored handle while holding the lock, so a
        // concurrent waiter blocks on the lock until completion instead of
        // seeing an empty slot and returning success before on_end has
        // returned. Clear the slot afterwards so a later wait is an idempotent
        // success. A JoinError means the watch task panicked (outside the
        // per-callback catch_unwind).
        let result = match guard.as_mut() {
            Some(handle) => runtime().block_on(&mut *handle),
            None => Ok(()),
        };
        *guard = None;
        drop(guard);
        match result {
            Ok(()) => AvisoOutcome::empty().into_raw(),
            Err(_) => error::panic_error().into_outcome(),
        }
    })
}

/// Frees a watch handle. Signals a stop first so a still-running task winds
/// down rather than running forever, then detaches it. Call `aviso_watch_wait`
/// before freeing if `on_end` may touch state you are about to release; freeing
/// alone does not wait for `on_end`. A null pointer is a no-op.
///
/// # Safety
///
/// `watch`, when non-null, must be a live handle from `aviso_client_watch` that
/// was not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_free(watch: *mut AvisoWatch) {
    if watch.is_null() {
        return;
    }
    // SAFETY: the pointer came from this library's constructor and is freed at
    // most once, per this function's # Safety.
    let watch = unsafe { Box::from_raw(watch) };
    watch.stop.flag.store(true, Ordering::Release);
    watch.stop.notify.notify_one();
    drop(watch);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "test code: every unsafe block here is a call into the crate's own C ABI from Rust, with the arguments constructed a few lines above"
)]
mod tests {
    use super::*;
    use crate::AvisoErrorKind;
    use crate::client::{aviso_client_builder_build, aviso_client_builder_new, aviso_client_free};
    use crate::outcome::{aviso_outcome_error, aviso_outcome_free, aviso_outcome_take_client};
    use std::ffi::{CStr, CString};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize};

    struct Sink {
        ended: AtomicBool,
        end_kind: AtomicI32,
        notifications: AtomicUsize,
        identifier_json: Mutex<Option<String>>,
    }

    impl Sink {
        fn new() -> Self {
            Self {
                ended: AtomicBool::new(false),
                end_kind: AtomicI32::new(-2),
                notifications: AtomicUsize::new(0),
                identifier_json: Mutex::new(None),
            }
        }
    }

    extern "C" fn on_notification(
        ctx: *mut c_void,
        notification: *const AvisoNotification,
    ) -> bool {
        let sink = unsafe { &*(ctx as *const Sink) };
        sink.notifications.fetch_add(1, Ordering::SeqCst);
        let identifier = unsafe { aviso_notification_identifier_json(notification) };
        if !identifier.is_null() {
            let text = unsafe { CStr::from_ptr(identifier) }
                .to_string_lossy()
                .into_owned();
            *sink.identifier_json.lock().expect("identifier lock") = Some(text);
        }
        true
    }

    extern "C" fn on_end(ctx: *mut c_void, outcome: *mut AvisoOutcome) {
        let sink = unsafe { &*(ctx as *const Sink) };
        let error = unsafe { aviso_outcome_error(outcome) };
        let kind = if error.is_null() {
            -1
        } else {
            unsafe { (*error).kind as i32 }
        };
        sink.end_kind.store(kind, Ordering::SeqCst);
        sink.ended.store(true, Ordering::SeqCst);
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

    fn cstr(value: &str) -> CString {
        CString::new(value).expect("cstring")
    }

    #[test]
    fn notification_callback_exposes_structured_identifier_json() {
        let notification = AvisoNotification {
            event_type: cstr("observations"),
            identifier_json: cstr(r#"{"point_cloud":[[46.0,8.0],[47.0,9.0]]}"#),
            payload_json: cstr("null"),
            sequence: 7,
        };
        let sink = Box::new(Sink::new());
        let ctx = (&raw const *sink) as *mut c_void;

        assert!(on_notification(ctx, &raw const notification));

        let identifier = sink.identifier_json.lock().expect("identifier lock");
        let parsed: serde_json::Value =
            serde_json::from_str(identifier.as_deref().expect("callback captured identifier"))
                .expect("identifier JSON");
        assert_eq!(
            parsed["point_cloud"],
            serde_json::json!([[46.0, 8.0], [47.0, 9.0]])
        );
    }

    #[test]
    fn watch_request_new_and_free_roundtrip() {
        let event = cstr("test_event");
        let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
        assert!(!request.is_null());
        unsafe { aviso_watch_request_free(request) };

        // A null event type still yields a freeable handle (the error is
        // remembered for the watch start).
        let bad = unsafe { aviso_watch_request_new(ptr::null()) };
        assert!(!bad.is_null());
        unsafe { aviso_watch_request_free(bad) };
    }

    #[test]
    fn client_watch_null_request_pointer_returns_null() {
        let watch = unsafe {
            aviso_client_watch(ptr::null(), ptr::null_mut(), None, None, ptr::null_mut())
        };
        assert!(watch.is_null());
    }

    #[test]
    fn client_watch_consumes_request_even_when_rejected() {
        // Null callbacks: the start is rejected, but the request must still be
        // taken and the caller's pointer nulled so a later free is a no-op.
        let event = cstr("test_event");
        let mut request = unsafe { aviso_watch_request_new(event.as_ptr()) };
        let watch = unsafe {
            aviso_client_watch(ptr::null(), &raw mut request, None, None, ptr::null_mut())
        };
        assert!(watch.is_null());
        assert!(request.is_null(), "the request pointer must be nulled");
    }

    #[test]
    fn watch_starts_then_stops_gracefully() {
        // No server: the supervisor keeps trying to connect to an unreachable
        // URL. A stop request must break the loop, close the stream, and fire
        // on_end exactly once with a graceful (no-error) outcome.
        let client = build_client();
        let event = cstr("test_event");
        let mut request = unsafe { aviso_watch_request_new(event.as_ptr()) };

        let sink = Box::new(Sink::new());
        let ctx = (&raw const *sink) as *mut c_void;

        let watch = unsafe {
            aviso_client_watch(
                client,
                &raw mut request,
                Some(on_notification),
                Some(on_end),
                ctx,
            )
        };
        assert!(!watch.is_null());
        assert!(request.is_null(), "the request pointer must be nulled");

        unsafe { aviso_watch_stop(watch) };
        let outcome = unsafe { aviso_watch_wait(watch) };
        assert!(!outcome.is_null());
        assert!(unsafe { aviso_outcome_error(outcome) }.is_null());
        unsafe { aviso_outcome_free(outcome) };

        assert!(sink.ended.load(Ordering::SeqCst), "on_end must have fired");
        assert_eq!(
            sink.end_kind.load(Ordering::SeqCst),
            -1,
            "a graceful stop must carry no error"
        );

        unsafe { aviso_watch_free(watch) };
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn add_trigger_consumes_and_nulls_the_handle() {
        let event = cstr("test_event");
        let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
        let mut trigger = crate::triggers::aviso_trigger_echo();
        unsafe { aviso_watch_request_add_trigger(request, &raw mut trigger) };
        assert!(trigger.is_null(), "the trigger pointer must be nulled");
        unsafe { aviso_watch_request_free(request) };
    }

    #[test]
    fn watch_with_a_bad_trigger_reports_invalid_input_via_on_end() {
        // A trigger built from a bad argument (a null log path) is remembered on
        // the request and surfaced through on_end when the watch starts, before
        // any stream work.
        let client = build_client();
        let event = cstr("test_event");
        let request = unsafe { aviso_watch_request_new(event.as_ptr()) };
        let mut trigger = unsafe { crate::triggers::aviso_trigger_log(ptr::null()) };
        unsafe { aviso_watch_request_add_trigger(request, &raw mut trigger) };

        let sink = Box::new(Sink::new());
        let ctx = (&raw const *sink) as *mut c_void;
        let mut request = request;
        let watch = unsafe {
            aviso_client_watch(
                client,
                &raw mut request,
                Some(on_notification),
                Some(on_end),
                ctx,
            )
        };
        assert!(!watch.is_null());
        let outcome = unsafe { aviso_watch_wait(watch) };
        unsafe { aviso_outcome_free(outcome) };
        assert!(sink.ended.load(Ordering::SeqCst));
        assert_eq!(
            sink.end_kind.load(Ordering::SeqCst),
            AvisoErrorKind::InvalidInput as i32
        );
        unsafe { aviso_watch_free(watch) };
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn watch_wait_is_idempotent_after_completion() {
        let client = build_client();
        let event = cstr("test_event");
        let mut request = unsafe { aviso_watch_request_new(event.as_ptr()) };
        let sink = Box::new(Sink::new());
        let ctx = (&raw const *sink) as *mut c_void;
        let watch = unsafe {
            aviso_client_watch(
                client,
                &raw mut request,
                Some(on_notification),
                Some(on_end),
                ctx,
            )
        };
        unsafe { aviso_watch_stop(watch) };
        let first = unsafe { aviso_watch_wait(watch) };
        unsafe { aviso_outcome_free(first) };
        // A second wait after completion returns success immediately.
        let second = unsafe { aviso_watch_wait(watch) };
        assert!(unsafe { crate::outcome::aviso_outcome_is_ok(second) });
        unsafe { aviso_outcome_free(second) };
        unsafe { aviso_watch_free(watch) };
        unsafe { aviso_client_free(client) };
    }
}
