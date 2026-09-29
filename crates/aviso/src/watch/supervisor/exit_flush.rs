// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Writing the last delivered notification to the state store when the
//! supervisor exits.

use std::sync::Arc;

use super::PendingCommit;
use crate::state::{Checkpoint, ResumeKey, StateStore};

/// Stores `pending` as the checkpoint for `resume_key` when the client asks
/// for it (`enabled`) and there is both a store and a pending notification.
/// A failure is logged, not returned: the supervisor is exiting, and the
/// only consequence is that the next run may deliver this notification
/// again.
pub(super) async fn flush_pending(
    enabled: bool,
    store: Option<&Arc<dyn StateStore>>,
    resume_key: &ResumeKey,
    pending: Option<&PendingCommit>,
) {
    let (true, Some(store), Some(pending)) = (enabled, store, pending) else {
        return;
    };
    let checkpoint = Checkpoint::new(pending.sequence, Some(pending.event_id.clone()));
    match store.put(resume_key, checkpoint).await {
        Ok(()) => {
            tracing::debug!(
                event.name = "client.resume.flushed_on_exit",
                resume_key = %resume_key.as_hex(),
                sequence = pending.sequence,
                event_id = %pending.event_id,
                "flushed pending commit to state store on supervisor exit",
            );
        }
        Err(e) => {
            tracing::warn!(
                event.name = "client.resume.flush_on_exit_failed",
                resume_key = %resume_key.as_hex(),
                sequence = pending.sequence,
                error = %e,
                "failed to flush pending commit on supervisor exit; the next run may redeliver this notification",
            );
        }
    }
}
