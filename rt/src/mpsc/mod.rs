//! Bounded MPSC channel traits.

mod buffered_receiver;
mod peekable_receiver;
mod send_error;
mod try_recv_error;

use std::future::Future;
use std::ops::RangeInclusive;

pub use buffered_receiver::BufferedReceiver;
use openraft_macros::since;
pub use peekable_receiver::PeekableReceiver;
pub use send_error::SendError;
pub use try_recv_error::TryRecvError;

use crate::AsyncRuntime;
use crate::OptionalSend;
use crate::OptionalSync;

/// Multi-producer, single-consumer channel.
#[since(version = "0.10.0", change = "remove the `OptionalSend` requirement")]
pub trait Mpsc: Sized {
    /// The sender type for this MPSC channel.
    type Sender<T: OptionalSend>: MpscSender<Self, T>;
    /// The receiver type for this MPSC channel.
    type Receiver<T: OptionalSend>: MpscReceiver<T>;
    /// The weak sender type for this MPSC channel.
    type WeakSender<T: OptionalSend>: MpscWeakSender<Self, T>;

    /// Creates a bounded mpsc channel for communicating between asynchronous tasks with
    /// backpressure.
    ///
    /// A `buffer` of at least 1, within the limit of the runtime, works on every runtime. The
    /// Tokio-backed channels of Tokio, Monoio and the simulator panic when `buffer` is 0 or
    /// greater than `tokio::sync::Semaphore::MAX_PERMITS`. Compio accepts 0 and creates a
    /// rendezvous channel, whose send completes only when a receiver takes the value.
    ///
    /// [`buffer_size_range()`](Self::buffer_size_range) gives the sizes that this runtime accepts.
    #[track_caller]
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>);

    /// The `buffer` sizes that [`channel()`](Self::channel) accepts.
    ///
    /// A caller can reject a size outside this range before `channel()` panics on it. The default
    /// is every size; an adapter whose channel rejects some sizes overrides it.
    #[since(version = "0.10.2")]
    fn buffer_size_range() -> RangeInclusive<usize> {
        0..=usize::MAX
    }
}

/// Send values to the associated [`MpscReceiver`].
pub trait MpscSender<MU, T>: OptionalSend + OptionalSync + Clone
where
    MU: Mpsc,
    T: OptionalSend,
{
    /// Attempts to send a message, blocks if there is no capacity.
    ///
    /// If the receiving half of the channel is closed, this
    /// function returns an error. The error includes the value passed to `send`.
    ///
    /// Dropping the returned future after it has been polled can race with delivery: on Compio
    /// the message can still reach the receiver, while Tokio, Monoio and the simulator drop it.
    /// A caller that sends the message again after a timeout or a lost selection can therefore
    /// deliver it twice. To finish the same send after another branch wins, keep the future and
    /// poll it again. A [`SendError`] is different: the message was not sent, and the error
    /// returns it.
    #[track_caller]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend;

    /// Converts the [`MpscSender`] to a [`MpscWeakSender`] that does not count
    /// towards RAII semantics, i.e., if all `Sender` instances of the
    /// channel were dropped and only `WeakSender` instances remain,
    /// the channel is closed.
    #[track_caller]
    fn downgrade(&self) -> MU::WeakSender<T>;
}

/// Receive values from the associated [`MpscSender`].
#[since(version = "0.10.0", change = "add an unbounded receiver buffer adapter")]
#[since(version = "0.10.0", change = "add peekable receiver conversion")]
#[since(version = "0.10.0", change = "remove the `OptionalSync` requirement")]
pub trait MpscReceiver<T>: OptionalSend {
    /// Receives the next value for this receiver.
    ///
    /// This method returns `None` if the channel has been closed and there are
    /// no remaining messages in the channel's buffer.
    #[track_caller]
    fn recv(&mut self) -> impl Future<Output = Option<T>> + OptionalSend;

    /// Tries to receive the next value for this receiver.
    ///
    /// This method returns the [`TryRecvError::Empty`] error if the channel is currently
    /// empty, but there are still outstanding senders.
    ///
    /// This method returns the [`TryRecvError::Disconnected`] error if the channel is
    /// currently empty, and there are no outstanding senders.
    #[track_caller]
    fn try_recv(&mut self) -> Result<T, TryRecvError>;

    /// Drain this receiver in a background task, buffering messages without a backlog limit.
    ///
    /// The producer can keep sending while the consumer is busy. Memory use grows with the
    /// backlog.
    ///
    /// [`try_recv`](Self::try_recv) on the returned receiver can return [`TryRecvError::Empty`]
    /// while the task still holds messages, or before the task has forwarded the closure of this
    /// receiver. Dropping the returned receiver cancels the task. This receiver is dropped later,
    /// when the runtime runs the cancelled task to its end, so a send can still succeed after the
    /// drop without being delivered. See [`BufferedReceiver`].
    #[since(version = "0.10.0")]
    fn with_unbounded_buffer<Rt>(self) -> BufferedReceiver<Rt, T>
    where
        Rt: AsyncRuntime,
        Self: Sized + 'static,
        T: OptionalSend + 'static,
    {
        BufferedReceiver::new(self)
    }

    /// Wraps this receiver to inspect the next item without consuming it or waiting.
    #[since(version = "0.10.0")]
    fn peekable(self) -> PeekableReceiver<Self, T>
    where
        Self: Sized,
        T: OptionalSend,
    {
        PeekableReceiver::new(self)
    }
}

/// A sender that does not prevent the channel from being closed.
///
/// If all [`MpscSender`] instances of a channel were dropped and only
/// `WeakSender` instances remain, the channel is closed.
pub trait MpscWeakSender<MU, T>: OptionalSend + OptionalSync + Clone
where
    MU: Mpsc,
    T: OptionalSend,
{
    /// Tries to convert a [`MpscWeakSender`] into an [`MpscSender`].
    ///
    /// This returns `Some` while another [`MpscSender`] of the channel is alive, and `None` once
    /// the last one has been dropped. Getting a sender says nothing about the receiver: when it
    /// has been dropped, [`send()`](MpscSender::send) on the upgraded sender returns
    /// [`SendError`] with the value.
    #[track_caller]
    fn upgrade(&self) -> Option<MU::Sender<T>>;
}
