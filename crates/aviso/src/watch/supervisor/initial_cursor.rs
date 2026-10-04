// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Where a watch starts.
//!
//! A start position in the request wins. Without one, a configured state
//! store is asked for the saved checkpoint; without a store, or without a
//! checkpoint, the watch starts at the live head.

use std::sync::Arc;

use tokio::sync::{oneshot, watch};

use crate::ClientError;
use crate::state::{ResumeKey, StateStore};
use crate::watch::{ResumeStart, WatchRequest};

/// How resolving the start position ended.
pub(super) enum InitialCursor {
    /// The position to start from; `None` is the live head.
    Resolved(Option<ResumeStart>),
    /// The stream was dropped while the store was being read.
    Cancelled,
    /// The store could not be read; the watch reports this and ends.
    Failed(ClientError),
}

/// Resolves the start position. The store read races both cancellation
/// signals, so dropping the stream during it ends the watch at once.
pub(super) async fn resolve(
    request: &WatchRequest,
    state_store: Option<&Arc<dyn StateStore>>,
    resume_key: &ResumeKey,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
) -> InitialCursor {
    if let Some(from) = request.from() {
        return InitialCursor::Resolved(Some(from.clone()));
    }
    let Some(store) = state_store else {
        return InitialCursor::Resolved(None);
    };
    let read = tokio::select! {
        biased;
        _ = parent_cancel.changed() => return InitialCursor::Cancelled,
        _ = &mut *cancel => return InitialCursor::Cancelled,
        read = store.get(resume_key) => read,
    };
    match read {
        Ok(Some(checkpoint)) => {
            // INFO level: resuming from saved state is worth an operator's
            // attention; starting fresh, the default, stays silent.
            tracing::info!(
                event.name = "client.resume.applied",
                resume_key = %resume_key.as_hex(),
                sequence = checkpoint.last_committed_sequence,
                event_id = checkpoint.last_event_id.as_deref(),
                "resumed watch from stored checkpoint",
            );
            InitialCursor::Resolved(Some(ResumeStart::AfterSequence(
                checkpoint.last_committed_sequence,
            )))
        }
        Ok(None) => InitialCursor::Resolved(None),
        Err(e) => InitialCursor::Failed(ClientError::from(e)),
    }
}
