use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};

use crate::client::decrement_active_key;
use crate::state::ResumeKey;
use crate::watch::{GapReason, ResumeStart};
use crate::{ClientError, Notification};

/// RAII guard that decrements the per-client `active_resume_keys`
/// refcount on supervisor exit, regardless of which exit path the
/// supervisor took (clean close, fatal error, cancellation, panic).
pub(crate) struct ActiveKeyGuard {
    pub(crate) active: Arc<std::sync::Mutex<std::collections::HashMap<ResumeKey, usize>>>,
    pub(crate) key: ResumeKey,
}

impl Drop for ActiveKeyGuard {
    fn drop(&mut self) {
        decrement_active_key(&self.active, &self.key);
    }
}

/// Send `msg` on `tx`, racing with both cancellation sources.
///
/// Returns `Ok(())` on successful send. Returns `Err(())` when any of these
/// fires first: the consumer dropped the receiver (`mpsc::SendError`), the
/// per-stream cancellation oneshot fired, or the parent-drop watch channel
/// flipped. All three cases mean "stop draining and exit cleanly".
///
/// Racing the parent-drop arm here matters when the channel is full: if a
/// supervisor is parked on `tx.send().await` because a slow consumer let
/// the bounded mpsc fill, dropping the parent `AvisoClient` must still
/// terminate the supervisor within one event-loop tick.
pub(crate) async fn send_or_cancel(
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    msg: Result<Notification, ClientError>,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
) -> Result<(), ()> {
    tokio::select! {
        biased;
        _ = parent_cancel.changed() => Err(()),
        _ = &mut *cancel => Err(()),
        result = tx.send(msg) => result.map_err(|_| ()),
    }
}

/// Sequence-jump detector. Holds the next expected sequence; updates on
/// every observation; reports a gap when the observed sequence is strictly
/// greater than expected.
///
/// Sequence going backwards (observed less than expected) is treated as a
/// duplicate or server-side anomaly and ignored at this layer; a future
/// commit may surface it as a structured WARN log.
pub(crate) struct GapGuard {
    expected: Option<u64>,
}

impl GapGuard {
    pub(super) fn starting_from(from: Option<&ResumeStart>) -> Self {
        let expected = match from {
            Some(ResumeStart::AfterSequence(n)) => n.checked_add(1),
            _ => None,
        };
        Self { expected }
    }

    pub(super) fn observe(&mut self, observed: u64) -> Result<(), GapReason> {
        match self.expected {
            None => {
                self.expected = observed.checked_add(1);
                Ok(())
            }
            Some(expected) if observed == expected => {
                self.expected = observed.checked_add(1);
                Ok(())
            }
            Some(expected) if observed > expected => {
                Err(GapReason::SequenceJump { expected, observed })
            }
            Some(_) => Ok(()),
        }
    }
}
