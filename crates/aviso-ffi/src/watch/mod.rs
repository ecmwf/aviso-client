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

// The declarations are in source order, not sorted: cbindgen writes the C
// header in this order, so it groups the request, the notification and the
// handle functions as a reader meets them. The blank line after each one is
// what keeps it there: rustfmt sorts adjacent `mod` lines.

mod request;

mod notification;

mod handle;

mod many;

#[cfg(test)]
mod tests;

pub use handle::AvisoWatch;
pub(crate) use handle::{StopSignal, deliver_end, deliver_raw_end, spawn_watch};
pub use many::AvisoWatchList;
pub use notification::AvisoNotification;
pub use request::AvisoWatchRequest;
