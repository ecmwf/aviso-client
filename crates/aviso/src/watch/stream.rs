// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! [`NotificationStream`]: the consumer's view of a live watch session.
//!
//! Backed by a bounded [`tokio::sync::mpsc`] channel that the supervisor
//! task fills. Dropping the stream signals cooperative cancellation to the
//! supervisor via a [`tokio::sync::oneshot`] channel held in the stream;
//! the supervisor `select!`s on the matching receiver and exits cleanly.
//! No `JoinHandle::abort` is involved.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use tokio::sync::{mpsc, oneshot};

use crate::{ClientError, Notification};

/// Asynchronous stream of [`Notification`] values from a single watch
/// session.
///
/// `NotificationStream` is single-consumer; it deliberately does not
/// implement [`Clone`]. Callers that want to fan a single watch out to
/// multiple consumers should tee through [`tokio::sync::broadcast`] (or
/// similar) themselves; the single-consumer contract keeps checkpoint
/// advancement and trigger semantics unambiguous.
///
/// The stream items are `Result<Notification, ClientError>` so transport,
/// decode, and stream-protocol failures surface inline rather than being
/// silently swallowed. After the first terminal error, the stream yields
/// `None` on subsequent polls; the supervisor task has already exited.
///
/// # Cancellation
///
/// Dropping the stream cancels the supervisor task cooperatively: the
/// internal cancellation oneshot's `Sender` is dropped, which closes the
/// matching `Receiver` the supervisor is `select!`-ing on. The supervisor
/// finishes its current syscall, observes the cancel, and exits within one
/// event-loop tick. No buffered notifications are lost from the consumer's
/// perspective because the consumer has already moved on.
///
/// # Backpressure
///
/// The internal channel is bounded at a fixed capacity of 128. A slow
/// consumer that lets the channel fill applies TCP backpressure all the
/// way upstream: the supervisor's `send` `await`s, which makes it stop
/// reading bytes from the wire, which makes the kernel stop `ACKing`,
/// which throttles the server. No notifications are dropped and no
/// internal buffer grows without bound.
#[non_exhaustive]
pub struct NotificationStream {
    receiver: mpsc::Receiver<Result<Notification, ClientError>>,
    /// Sender side of the cancellation oneshot. Optional so
    /// [`NotificationStream::close`] can drop it BEFORE awaiting
    /// `done_signal`, while still letting RAII drop work for callers
    /// that never call `close`.
    cancel: Option<oneshot::Sender<()>>,
    /// Receiver side of the supervisor's done-signal oneshot. The
    /// supervisor sends `()` on this channel immediately before its
    /// task returns (via a `Drop` guard so the signal fires on every
    /// exit path, including panics). Consumers that need to wait for
    /// the supervisor's post-loop work (notably the
    /// `flush_cursor_on_exit` path that writes the final checkpoint to
    /// the [`crate::state::StateStore`]) call
    /// [`NotificationStream::close`] which awaits this receiver.
    done_signal: Option<oneshot::Receiver<()>>,
    ready: tokio::sync::watch::Receiver<bool>,
}

impl NotificationStream {
    /// Construct a `NotificationStream` with its lifecycle signals. Only the
    /// supervisor-setup path in [`crate::client::AvisoClient::watch`]
    /// builds streams; this constructor is `pub(crate)` and not part of
    /// the public surface.
    pub(crate) fn new(
        receiver: mpsc::Receiver<Result<Notification, ClientError>>,
        cancel: oneshot::Sender<()>,
        done_signal: oneshot::Receiver<()>,
        ready: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        Self {
            receiver,
            cancel: Some(cancel),
            done_signal: Some(done_signal),
            ready,
        }
    }

    /// Await and return the next stream item.
    ///
    /// `Some(Ok(_))` is a notification; `Some(Err(_))` is a terminal
    /// error followed by `None` on the next call; `None` means the stream
    /// has ended cleanly (server-driven close, drop, or supervisor exit).
    ///
    /// Equivalent to one `<Self as futures_core::Stream>::poll_next` cycle
    /// driven to readiness. Provided so callers who do not want to pull in
    /// `futures-util` for `StreamExt::next` can drain the stream with a
    /// plain `await`.
    pub async fn recv(&mut self) -> Option<Result<Notification, ClientError>> {
        self.receiver.recv().await
    }

    /// Subscribe to first-handshake confirmation, without waiting for data.
    /// The value starts false and stays true after the first validated Aviso
    /// opening event, including during reconnects. Channel closure while false
    /// means startup ended; read the stream for the terminal error.
    #[must_use]
    pub fn subscribe_ready(&self) -> tokio::sync::watch::Receiver<bool> {
        self.ready.clone()
    }

    /// Cancel the supervisor cooperatively and wait for it to fully
    /// exit (including any post-loop work such as the
    /// [`AvisoClientBuilder::flush_cursor_on_exit`](crate::AvisoClientBuilder::flush_cursor_on_exit)
    /// checkpoint flush).
    ///
    /// Use this instead of plain RAII drop when the supervisor has
    /// configured `flush_cursor_on_exit = true` AND the caller is about
    /// to drop the runtime (typical for short-lived CLI processes): a
    /// bare drop signals cancel but does NOT keep the runtime alive
    /// long enough for the supervisor's final
    /// [`StateStore::put`](crate::state::StateStore::put) (which uses
    /// [`tokio::task::spawn_blocking`] internally) to land on disk, so
    /// the next run silently re-delivers the last notification of the
    /// previous run.
    ///
    /// Callers that only need at-least-once redelivery semantics
    /// (`flush_cursor_on_exit = false`, the library default) get the
    /// same behaviour from a plain drop and do not need to call this.
    ///
    /// `close` takes `self` by value so the supervisor is guaranteed to
    /// observe the cancel (the cancel oneshot's sender is dropped here)
    /// and so the await of `done_signal` cannot be raced against a
    /// subsequent `recv`.
    ///
    /// Returns immediately (without awaiting) if the supervisor has
    /// already signalled completion (e.g., the consumer drained the
    /// stream to its natural end and the server emitted a
    /// `connection-closing` frame before `close` was reached).
    pub async fn close(mut self) {
        self.cancel.take();
        if let Some(done) = self.done_signal.take() {
            let _ = done.await;
        }
    }
}

impl std::fmt::Debug for NotificationStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationStream").finish_non_exhaustive()
    }
}

impl Stream for NotificationStream {
    type Item = Result<Notification, ClientError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.receiver.poll_recv(cx)
    }
}
