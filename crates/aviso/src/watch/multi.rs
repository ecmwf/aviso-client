// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Several watches read through one stream.
//!
//! [`crate::AvisoClient::watch_many`] opens one watch per named request and
//! merges them into a [`MultiNotificationStream`]. Each item carries the name
//! of the watch it came from. Watches are polled in turn, starting after the
//! one that last produced an item, so a busy watch cannot starve a quiet one.
//! The stream ends when every watch has ended.
//!
//! A watch that fails yields one [`EntryError`] naming it. What happens to
//! the others is the [`ErrorPolicy`]: [`ErrorPolicy::Stop`] ends them all,
//! [`ErrorPolicy::Continue`] drops the failed watch and keeps reading the
//! rest.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

use super::NotificationStream;
use crate::{ClientError, Notification};

/// What a [`MultiNotificationStream`] does when one of its watches fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ErrorPolicy {
    /// Yield the error, then end: the other watches are closed.
    #[default]
    Stop,
    /// Yield the error, drop that watch, and keep reading the others.
    Continue,
}

/// A watch's terminal error, with the name of the watch.
#[derive(Debug)]
#[non_exhaustive]
pub struct EntryError {
    /// The name the watch was given in [`crate::AvisoClient::watch_many`].
    pub name: String,
    /// What went wrong.
    pub error: ClientError,
}

impl std::fmt::Display for EntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "watch '{}': {}", self.name, self.error)
    }
}

impl std::error::Error for EntryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Several watches merged into one stream of named notifications.
///
/// Items are `Ok((name, notification))` or `Err(EntryError)`. The stream
/// ends (`None`) when every watch has ended, or after an error under
/// [`ErrorPolicy::Stop`]. Dropping it cancels every watch;
/// [`Self::close`] also waits for them to finish.
pub struct MultiNotificationStream {
    entries: Vec<Entry>,
    /// Watches that no longer produce items: ended, failed, or stopped by
    /// [`ErrorPolicy::Stop`]. Kept so [`Self::close`] waits for each to
    /// finish, including any final checkpoint, as it would for one watch.
    stopped: Vec<NotificationStream>,
    policy: ErrorPolicy,
    /// Where the next poll starts, so the watch after the last producer
    /// goes first.
    next: usize,
}

struct Entry {
    name: String,
    /// `None` once the watch has ended or failed.
    stream: Option<NotificationStream>,
}

impl MultiNotificationStream {
    pub(crate) fn new(streams: Vec<(String, NotificationStream)>, policy: ErrorPolicy) -> Self {
        Self {
            entries: streams
                .into_iter()
                .map(|(name, stream)| Entry {
                    name,
                    stream: Some(stream),
                })
                .collect(),
            policy,
            next: 0,
            stopped: Vec::new(),
        }
    }

    /// The names of the watches still running, in the order they were given.
    #[must_use]
    pub fn running(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.stream.is_some())
            .map(|e| e.name.as_str())
            .collect()
    }

    /// Closes every watch and waits for each to finish, as
    /// [`NotificationStream::close`] does for one. All are cancelled first,
    /// so they shut down together rather than one after another.
    pub async fn close(mut self) {
        for entry in &mut self.entries {
            if let Some(mut stream) = entry.stream.take() {
                stream.cancel();
                self.stopped.push(stream);
            }
        }
        for stream in self.stopped.drain(..) {
            stream.close().await;
        }
    }
}

impl Stream for MultiNotificationStream {
    type Item = Result<(String, Notification), EntryError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        let count = this.entries.len();
        for offset in 0..count {
            let index = (this.next + offset) % count;
            let Some(entry) = this.entries.get_mut(index) else {
                continue;
            };
            let Some(stream) = entry.stream.as_mut() else {
                continue;
            };
            match Pin::new(stream).poll_next(cx) {
                Poll::Pending => {}
                Poll::Ready(Some(Ok(notification))) => {
                    this.next = (index + 1) % count;
                    return Poll::Ready(Some(Ok((entry.name.clone(), notification))));
                }
                Poll::Ready(Some(Err(error))) => {
                    // A watch yields one error and then ends; keep it so
                    // close() still waits for its final checkpoint.
                    if let Some(ended) = entry.stream.take() {
                        this.stopped.push(ended);
                    }
                    let name = entry.name.clone();
                    this.next = (index + 1) % count;
                    if this.policy == ErrorPolicy::Stop {
                        // Cancel the others now; keep them so close() can
                        // wait for them to finish.
                        for other in &mut this.entries {
                            if let Some(mut stream) = other.stream.take() {
                                stream.cancel();
                                this.stopped.push(stream);
                            }
                        }
                    }
                    return Poll::Ready(Some(Err(EntryError { name, error })));
                }
                Poll::Ready(None) => {
                    if let Some(ended) = entry.stream.take() {
                        this.stopped.push(ended);
                    }
                }
            }
        }
        if this.entries.iter().all(|e| e.stream.is_none()) {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

impl std::fmt::Debug for MultiNotificationStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiNotificationStream")
            .field("running", &self.running())
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap on known-good fixtures is the expected diagnostic"
)]
mod tests {
    use futures_util::StreamExt;
    use tokio::sync::{mpsc, oneshot, watch};

    use super::*;

    fn notification(sequence: u64) -> Notification {
        Notification {
            event_type: "mars".into(),
            sequence,
            identifier: std::collections::BTreeMap::new(),
            payload: serde_json::Value::Null,
            cloudevent: None,
        }
    }

    type Sender = mpsc::Sender<Result<Notification, ClientError>>;

    /// A watch whose items are already buffered: the channel is filled
    /// before the stream is read, so nothing depends on timing. The sender
    /// is returned so the caller keeps the stream open once it is drained.
    fn buffered(items: u64) -> (NotificationStream, Sender) {
        let (tx, rx) = mpsc::channel(128);
        for sequence in 1..=items {
            tx.try_send(Ok(notification(sequence))).unwrap();
        }
        let (cancel, _cancel_rx) = oneshot::channel();
        let (_done, done_rx) = oneshot::channel();
        let (_ready, ready_rx) = watch::channel(true);
        (NotificationStream::new(rx, cancel, done_rx, ready_rx), tx)
    }

    #[tokio::test]
    async fn a_busy_watch_does_not_starve_a_quiet_one() {
        let (busy, _busy_tx) = buffered(50);
        let (quiet, _quiet_tx) = buffered(1);
        let mut stream = MultiNotificationStream::new(
            vec![("busy".into(), busy), ("quiet".into(), quiet)],
            ErrorPolicy::Stop,
        );
        let mut names = Vec::new();
        for _ in 0..4 {
            let (name, _) = stream.next().await.unwrap().unwrap();
            names.push(name);
        }
        // Turns alternate while both have items; the quiet watch's one item
        // comes second, not after the busy watch's fifty.
        assert_eq!(names, ["busy", "quiet", "busy", "busy"]);
    }
}
