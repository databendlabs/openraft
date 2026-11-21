use std::future::Future;
use std::ops::RangeInclusive;

use futures_util::TryFutureExt;
use openraft_rt::Mpsc;
use openraft_rt::MpscReceiver;
use openraft_rt::MpscSender;
use openraft_rt::MpscWeakSender;
use openraft_rt::OptionalSend;
use openraft_rt::SendError;
use openraft_rt::TryRecvError;
use tokio::sync::mpsc;

pub struct TokioMpsc;

/// Wrapper around `tokio::sync::mpsc::Sender` to implement the `MpscSender` trait.
pub struct TokioMpscSender<T>(mpsc::Sender<T>);

/// Wrapper around `tokio::sync::mpsc::Receiver` to implement the `MpscReceiver` trait.
pub struct TokioMpscReceiver<T>(mpsc::Receiver<T>);

/// Wrapper around `tokio::sync::mpsc::WeakSender` to implement the `MpscWeakSender` trait.
pub struct TokioMpscWeakSender<T>(mpsc::WeakSender<T>);

impl<T> Clone for TokioMpscSender<T> {
    fn clone(&self) -> Self {
        TokioMpscSender(self.0.clone())
    }
}

impl<T> Clone for TokioMpscWeakSender<T> {
    fn clone(&self) -> Self {
        TokioMpscWeakSender(self.0.clone())
    }
}

impl Mpsc for TokioMpsc {
    type Sender<T: OptionalSend> = TokioMpscSender<T>;
    type Receiver<T: OptionalSend> = TokioMpscReceiver<T>;
    type WeakSender<T: OptionalSend> = TokioMpscWeakSender<T>;

    /// Creates a bounded mpsc channel for communicating between asynchronous
    /// tasks with backpressure.
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = mpsc::channel(buffer);
        (TokioMpscSender(tx), TokioMpscReceiver(rx))
    }

    /// Accepts 1 to `tokio::sync::Semaphore::MAX_PERMITS`: a Tokio channel panics on 0 and on a
    /// larger size.
    fn buffer_size_range() -> RangeInclusive<usize> {
        1..=tokio::sync::Semaphore::MAX_PERMITS
    }
}

impl<T> MpscSender<TokioMpsc, T> for TokioMpscSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend {
        self.0.send(msg).map_err(|e| SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <TokioMpsc as Mpsc>::WeakSender<T> {
        TokioMpscWeakSender(self.0.downgrade())
    }
}

impl<T> MpscReceiver<T> for TokioMpscReceiver<T>
where T: OptionalSend
{
    #[inline]
    fn recv(&mut self) -> impl Future<Output = Option<T>> + OptionalSend {
        self.0.recv()
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.0.try_recv().map_err(|e| match e {
            mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
            mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }
}

impl<T> MpscWeakSender<TokioMpsc, T> for TokioMpscWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<TokioMpsc as Mpsc>::Sender<T>> {
        self.0.upgrade().map(TokioMpscSender)
    }
}

#[cfg(test)]
mod tests {
    use openraft_rt::Mpsc;

    use super::TokioMpsc;

    #[test]
    fn test_buffer_size_range_is_what_a_tokio_channel_accepts() {
        let range = TokioMpsc::buffer_size_range();
        assert_eq!(1..=tokio::sync::Semaphore::MAX_PERMITS, range);

        let _channel = TokioMpsc::channel::<u32>(*range.start());
        let _channel = TokioMpsc::channel::<u32>(*range.end());

        let below = std::panic::catch_unwind(|| TokioMpsc::channel::<u32>(range.start() - 1));
        assert!(below.is_err(), "a Tokio channel panics below the range");
        let above = std::panic::catch_unwind(|| TokioMpsc::channel::<u32>(range.end() + 1));
        assert!(above.is_err(), "a Tokio channel panics above the range");
    }
}
