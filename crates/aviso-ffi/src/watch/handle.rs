// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The running watch: the task that drives a stream and calls the C
//! callbacks, the handle the caller stops, waits for and frees, and
//! `aviso_client_watch`, which starts a single watch.

use std::ffi::c_void;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aviso::ClientError;
use aviso::watch::WatchRequest;
use tokio::sync::Notify;

use super::notification::AvisoNotification;
use super::request::AvisoWatchRequest;
use crate::client::AvisoClient;
use crate::error;
use crate::outcome::AvisoOutcome;
use crate::send::{SendOutcome, SendPtr};
use crate::{guard, guard_outcome, reject_blocking_on_runtime, runtime};

/// C callback invoked once per notification. Returning `false` requests a
/// graceful stop; the borrowed `notification` is valid only for the call.
type OnNotification =
    extern "C" fn(ctx: *mut c_void, notification: *const AvisoNotification) -> bool;

/// C callback invoked exactly once when the watch ends or fails. It takes
/// ownership of `outcome` and must free it with `aviso_outcome_free`.
pub(crate) type OnEnd = extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome);

/// Cooperative stop signal shared by the watch handle and its task.
pub(crate) struct StopSignal {
    pub(crate) flag: AtomicBool,
    pub(crate) notify: Notify,
}

/// Spawns a watch task on the global runtime and returns its handle. `task`
/// receives the stop signal that `aviso_watch_stop` and `aviso_watch_free`
/// raise, and must end soon after it is raised.
pub(crate) fn spawn_watch<F, Fut>(task: F) -> *mut AvisoWatch
where
    F: FnOnce(Arc<StopSignal>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let stop = Arc::new(StopSignal {
        flag: AtomicBool::new(false),
        notify: Notify::new(),
    });
    let join = runtime().spawn(task(Arc::clone(&stop)));
    Box::into_raw(Box::new(AvisoWatch {
        stop,
        join: std::sync::Mutex::new(Some(join)),
    }))
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
pub(crate) fn deliver_end(on_end: OnEnd, ctx: SendPtr, outcome: AvisoOutcome) {
    deliver_raw_end(on_end, ctx, outcome.into_raw());
}

/// As [`deliver_end`], for an outcome already handed out as a raw pointer.
pub(crate) fn deliver_raw_end(on_end: OnEnd, ctx: SendPtr, outcome: *mut AvisoOutcome) {
    // reason: on_end returns nothing, so a trapped unwind has no one to be
    // reported to; trapping it keeps it from crossing into the runtime.
    catch_unwind(AssertUnwindSafe(|| on_end(ctx.0, outcome))).ok();
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
        let send_ctx = SendPtr(ctx);

        spawn_watch(move |stop| async move {
            let send_ctx = send_ctx;
            let request = match built {
                Ok(request) => request,
                Err(send_outcome) => {
                    deliver_raw_end(on_end, send_ctx, send_outcome.0);
                    return;
                }
            };
            let outcome = run_watch(client, request, on_notification, send_ctx, &stop).await;
            deliver_end(on_end, send_ctx, outcome);
        })
    })
}

/// Requests a graceful stop. Nonblocking, idempotent, and safe to call from a
/// callback. A callback already running finishes; no new callback starts after
/// the stop is observed. `on_end` still fires once. A null pointer is a no-op.
///
/// # Safety
///
/// `watch`, when non-null, must be a live handle from `aviso_client_watch` or
/// `aviso_client_watch_many`.
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
/// `watch` must be a live handle from `aviso_client_watch` or
/// `aviso_client_watch_many`.
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
/// `watch`, when non-null, must be a live handle from `aviso_client_watch` or
/// `aviso_client_watch_many` that was not already freed.
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
