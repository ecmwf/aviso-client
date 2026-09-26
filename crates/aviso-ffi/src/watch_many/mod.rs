// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The merged watch surface: several named watch requests read through one
//! watch handle.
//!
//! `aviso_client_watch_many` spawns one task that opens every request with
//! the core `AvisoClient::watch_many` and reads the merged stream. Each
//! notification reaches `on_notification` with the name of its watch. When a
//! watch fails, `on_error` receives its name and error and decides what
//! happens next: `true` drops that watch and keeps reading the others,
//! `false` stops every watch. `on_end` fires once, when every watch has
//! ended or the merged watch was stopped.
//!
//! The core stream always runs under `ErrorPolicy::Continue`; the decision to
//! stop is the caller's, taken in `on_error`. The returned handle is an
//! `AvisoWatch`, so stop, wait and free work as for a single watch.

use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use aviso::watch::{EntryError, ErrorPolicy, WatchRequest};
use futures_util::StreamExt;

use crate::client::{AvisoClient, cstr_opt};
use crate::error::{self, AvisoError, OutcomeError};
use crate::guard;
use crate::outcome::AvisoOutcome;
use crate::send::{SendOutcome, SendPtr};
use crate::watch::{
    AvisoNotification, AvisoWatch, AvisoWatchRequest, StopSignal, deliver_end, deliver_raw_end,
    spawn_watch,
};

#[cfg(test)]
mod tests;

/// C callback invoked once per notification with the name of its watch.
/// Returning `false` stops every watch. `name` and `notification` are
/// borrowed and valid only for the call.
type OnNamedNotification = extern "C" fn(
    ctx: *mut c_void,
    name: *const c_char,
    notification: *const AvisoNotification,
) -> bool;

/// C callback invoked when one watch fails. Returning `true` drops that watch
/// and keeps the others; returning `false` stops every watch. `name` and
/// `error` are borrowed and valid only for the call.
type OnEntryError =
    extern "C" fn(ctx: *mut c_void, name: *const c_char, error: *const AvisoError) -> bool;

/// Opaque list of named watch requests for `aviso_client_watch_many`. The
/// first bad argument is remembered and surfaced through `on_end` when the
/// watch starts. Consumed by `aviso_client_watch_many` (which nulls the
/// caller's pointer) or freed with `aviso_watch_list_free`.
pub struct AvisoWatchList {
    entries: Vec<(String, WatchRequest)>,
    error: Option<OutcomeError>,
}

/// Prefixes an error message with the name of the watch it belongs to, as
/// the core does for its own errors: `watch 'surface': http 400: ...`.
fn named_message(name: &str, message: &str) -> String {
    format!("watch '{name}': {message}")
}

/// Creates an empty watch list. Returns null only if an internal panic is
/// trapped. Free an abandoned list with `aviso_watch_list_free`.
#[unsafe(no_mangle)]
pub extern "C" fn aviso_watch_list_new() -> *mut AvisoWatchList {
    guard(ptr::null_mut(), || {
        Box::into_raw(Box::new(AvisoWatchList {
            entries: Vec::new(),
            error: None,
        }))
    })
}

/// Adds the request `*request` to the list under `name`, consuming it: the
/// caller's pointer is nulled, even when the list is null or the call is
/// rejected. A null or non-UTF-8 `name`, a null request, or an error
/// remembered on the request is remembered on the list and surfaced when the
/// watch starts; later additions are then ignored. Empty and repeated names
/// are reported the same way when the watch starts.
///
/// # Safety
///
/// `list`, when non-null, must be a live handle from `aviso_watch_list_new`.
/// `name`, when non-null, must be a NUL-terminated C string. `request`, when
/// non-null, must point to a request-handle pointer; `*request`, when
/// non-null, must be a live handle from `aviso_watch_request_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_list_add(
    list: *mut AvisoWatchList,
    name: *const c_char,
    request: *mut *mut AvisoWatchRequest,
) {
    guard((), || {
        // Take the request first so it is consumed on every path.
        let owned = if request.is_null() {
            None
        } else {
            // SAFETY: `request` is non-null (checked above) and, per this
            // function's # Safety, points to a valid, aligned handle slot
            // that nothing else touches during this call.
            let slot = unsafe { &mut *request };
            if slot.is_null() {
                None
            } else {
                // SAFETY: the slot holds a handle this library created and
                // has not freed; ownership passes back here once and the slot
                // is nulled right after, per this function's # Safety.
                let owned = unsafe { Box::from_raw(*slot) };
                *slot = ptr::null_mut();
                Some(owned)
            }
        };
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_mut returns
        // None for null.
        let Some(list) = (unsafe { list.as_mut() }) else {
            return;
        };
        if list.error.is_some() {
            return;
        }
        // SAFETY: per this function's # Safety, `name` is null or a
        // NUL-terminated C string that outlives this call; the borrow is
        // copied below and not kept.
        let Some(name) = (unsafe { cstr_opt(name) }) else {
            list.error = Some(error::invalid_input(
                "a watch name must be a non-null UTF-8 string",
            ));
            return;
        };
        let Some(owned) = owned else {
            list.error = Some(error::invalid_input(&named_message(
                name,
                "the request must not be null",
            )));
            return;
        };
        match (owned.error, owned.spec) {
            (Some(err), _) => {
                let message = named_message(name, &err.message_string());
                list.error = Some(err.with_message(message));
            }
            (None, Some(spec)) => list.entries.push((name.to_string(), spec.into_request())),
            (None, None) => {
                list.error = Some(error::internal(&named_message(
                    name,
                    "the request was already consumed",
                )));
            }
        }
    });
}

/// Frees a watch list that was not handed to `aviso_client_watch_many`,
/// with the requests it holds. A null pointer is a no-op.
///
/// # Safety
///
/// `list`, when non-null, must be a live handle from `aviso_watch_list_new`
/// that was not already freed or consumed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_watch_list_free(list: *mut AvisoWatchList) {
    if list.is_null() {
        return;
    }
    // SAFETY: the pointer came from this library's constructor and is freed
    // at most once, per this function's # Safety.
    drop(unsafe { Box::from_raw(list) });
}

/// Why a merged watch ended. Holds core errors (which are `Send`) rather
/// than built `OutcomeError`s (which are not), so it can be carried across
/// the `close().await` and mapped afterwards.
enum EndReason {
    /// Every watch ended cleanly, some failed and were dropped, or a stop
    /// was requested.
    Graceful,
    /// `on_error` returned `false` for this failure.
    Stopped(EntryError),
    /// Every watch failed and `on_error` kept going each time.
    AllFailed(Vec<EntryError>),
    Panicked,
}

/// The error for one failed watch, named, as `on_error` and `on_end` see it.
fn entry_error(failure: &EntryError) -> OutcomeError {
    let mapped = error::map_error(&failure.error);
    let message = named_message(&failure.name, &mapped.message_string());
    mapped.with_message(message)
}

/// The error for a merged watch whose watches all failed. It keeps the kind,
/// status and request id of the last failure; the message lists every one.
fn all_failed_error(failures: &[EntryError]) -> OutcomeError {
    let messages: Vec<String> = failures
        .iter()
        .map(|failure| entry_error(failure).message_string())
        .collect();
    let message = format!("every watch failed: {}", messages.join("; "));
    match failures.last() {
        Some(last) => entry_error(last).with_message(message),
        None => error::internal(&message),
    }
}

/// Drives a merged watch to completion, returning the outcome for `on_end`.
async fn run_watch_many(
    client: aviso::AvisoClient,
    entries: Vec<(String, WatchRequest)>,
    on_notification: OnNamedNotification,
    on_error: OnEntryError,
    ctx: SendPtr,
    stop: &StopSignal,
) -> AvisoOutcome {
    let count = entries.len();
    let mut stream = match client.watch_many(entries, ErrorPolicy::Continue) {
        Ok(stream) => stream,
        Err(err) => return AvisoOutcome::error(error::map_error(&err)),
    };

    let mut failures: Vec<EntryError> = Vec::new();
    let reason = loop {
        if stop.flag.load(std::sync::atomic::Ordering::Acquire) {
            break EndReason::Graceful;
        }
        tokio::select! {
            biased;
            () = stop.notify.notified() => break EndReason::Graceful,
            item = stream.next() => match item {
                Some(Ok((name, notification))) => {
                    let name = error::cstring_lossy(name);
                    let view = AvisoNotification::from_core(&notification);
                    let view_ptr = &raw const view;
                    let call = || on_notification(ctx.0, name.as_ptr(), view_ptr);
                    match catch_unwind(AssertUnwindSafe(call)) {
                        Ok(true) => {}
                        Ok(false) => break EndReason::Graceful,
                        Err(_) => break EndReason::Panicked,
                    }
                }
                Some(Err(failure)) => {
                    // A copy: `failure` itself is kept below, as the stop
                    // reason or in the list of failures.
                    let name = error::cstring_lossy(failure.name.clone());
                    let reported = entry_error(&failure);
                    let call = || on_error(ctx.0, name.as_ptr(), reported.view());
                    match catch_unwind(AssertUnwindSafe(call)) {
                        Ok(true) => failures.push(failure),
                        Ok(false) => break EndReason::Stopped(failure),
                        Err(_) => break EndReason::Panicked,
                    }
                }
                None if failures.len() == count => {
                    break EndReason::AllFailed(std::mem::take(&mut failures));
                }
                None => break EndReason::Graceful,
            },
        }
    };

    // Close (rather than drop) so every watch's final cursor flush lands
    // before the task returns.
    stream.close().await;

    match reason {
        EndReason::Graceful => AvisoOutcome::empty(),
        EndReason::Stopped(failure) => AvisoOutcome::error(entry_error(&failure)),
        EndReason::AllFailed(failures) => AvisoOutcome::error(all_failed_error(&failures)),
        EndReason::Panicked => AvisoOutcome::error(error::panic_error()),
    }
}

/// Starts a merged watch over every request in the list, consuming the list.
/// It runs on the global runtime and returns at once.
///
/// - `on_notification` fires per notification with the name of its watch.
///   Returning `false` stops every watch.
/// - `on_error` fires when one watch fails, with its name and error; the
///   message begins with `watch '<name>': `. Returning `true` drops that
///   watch and keeps reading the others; returning `false` stops every watch
///   and `on_end` then receives the same error.
/// - `on_end` fires exactly once and takes ownership of the outcome. It
///   receives success when every watch has ended, including after failures
///   `on_error` chose to continue past, or after a stop. It receives an error
///   when the list was invalid (no requests, an empty or repeated name, a
///   bad request), when `on_error` returned `false`, or when every watch
///   failed; the last carries the kind of the last failure and a message
///   listing each one.
///
/// Watches are read in turn, so a busy watch cannot delay a quiet one. The
/// callbacks run on a runtime thread, one at a time for this watch, so they
/// must be thread-safe, must not unwind across the boundary, and must not
/// make blocking aviso calls. Stop, wait for and free the returned handle
/// with `aviso_watch_stop`, `aviso_watch_wait` and `aviso_watch_free`; they
/// act on every watch.
///
/// Returns null when `client`, the list, or a callback is null (no watch
/// starts), or if an internal panic is trapped. On entry the list is taken
/// and the caller's pointer is nulled, so a later free is a safe no-op.
///
/// # Safety
///
/// `client` must be a live handle from a successful build. `list` must point
/// to a list-handle pointer; `*list`, when non-null, must be a live handle
/// from `aviso_watch_list_new`. `ctx` is passed verbatim to the callbacks and
/// must stay valid until `on_end` has returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_watch_many(
    client: *const AvisoClient,
    list: *mut *mut AvisoWatchList,
    on_notification: Option<
        extern "C" fn(
            ctx: *mut c_void,
            name: *const c_char,
            notification: *const AvisoNotification,
        ) -> bool,
    >,
    on_error: Option<
        extern "C" fn(ctx: *mut c_void, name: *const c_char, error: *const AvisoError) -> bool,
    >,
    on_end: Option<extern "C" fn(ctx: *mut c_void, outcome: *mut AvisoOutcome)>,
    ctx: *mut c_void,
) -> *mut AvisoWatch {
    guard(ptr::null_mut(), || {
        if list.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: `list` is non-null (checked above) and, per this function's
        // # Safety, points to a valid, aligned handle slot that nothing else
        // touches during this call.
        let slot = unsafe { &mut *list };
        if slot.is_null() {
            return ptr::null_mut();
        }
        // Take ownership and null the caller's pointer before any validation,
        // so a later free is a safe no-op even when the start is rejected.
        // SAFETY: the slot holds a handle this library created and has not
        // freed; ownership passes back here once and the slot is nulled right
        // after, per this function's # Safety.
        let owned = unsafe { Box::from_raw(*slot) };
        *slot = ptr::null_mut();

        let (Some(on_notification), Some(on_error), Some(on_end)) =
            (on_notification, on_error, on_end)
        else {
            return ptr::null_mut();
        };
        // SAFETY: the handle is null or one this library handed out and the
        // caller has not freed, per this function's # Safety; as_ref returns
        // None for null.
        let Some(client) = (unsafe { client.as_ref() }) else {
            return ptr::null_mut();
        };

        // Build a start error into an owned outcome now (a Send pointer), so
        // the task captures only Send values.
        let AvisoWatchList { entries, error } = *owned;
        let start: Result<Vec<(String, WatchRequest)>, SendOutcome> = match error {
            Some(err) => Err(SendOutcome(AvisoOutcome::error(err).into_raw())),
            None => Ok(entries),
        };

        let client = client.inner.clone();
        let send_ctx = SendPtr(ctx);
        spawn_watch(move |stop| async move {
            let send_ctx = send_ctx;
            let entries = match start {
                Ok(entries) => entries,
                Err(outcome) => {
                    deliver_raw_end(on_end, send_ctx, outcome.0);
                    return;
                }
            };
            let outcome =
                run_watch_many(client, entries, on_notification, on_error, send_ctx, &stop).await;
            deliver_end(on_end, send_ctx, outcome);
        })
    })
}
