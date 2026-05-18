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
    /// Sender side of the cancellation oneshot. Held only to drop it; the
    /// supervisor owns the receiver side. Underscored to silence the
    /// "field is never read" lint without hiding the intent.
    _cancel: oneshot::Sender<()>,
}

impl NotificationStream {
    /// Construct a `NotificationStream` from its two halves. Only the
    /// supervisor-setup path in [`crate::client::AvisoClient::watch`]
    /// builds streams; this constructor is `pub(crate)` and not part of
    /// the public surface.
    pub(crate) fn new(
        receiver: mpsc::Receiver<Result<Notification, ClientError>>,
        cancel: oneshot::Sender<()>,
    ) -> Self {
        Self {
            receiver,
            _cancel: cancel,
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
