//! MPSC channel is implemented with tokio MPSC channels.
//!
//! Tokio MPSC channel are runtime independent.

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
use tokio::sync::mpsc as tokio_mpsc;

pub struct MonoioMpsc;

pub struct MonoioMpscSender<T>(tokio_mpsc::Sender<T>);

impl<T> Clone for MonoioMpscSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct MonoioMpscReceiver<T>(tokio_mpsc::Receiver<T>);

pub struct MonoioMpscWeakSender<T>(tokio_mpsc::WeakSender<T>);

impl<T> Clone for MonoioMpscWeakSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Mpsc for MonoioMpsc {
    type Sender<T: OptionalSend> = MonoioMpscSender<T>;
    type Receiver<T: OptionalSend> = MonoioMpscReceiver<T>;
    type WeakSender<T: OptionalSend> = MonoioMpscWeakSender<T>;

    #[inline]
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = tokio_mpsc::channel(buffer);
        let tx_wrapper = MonoioMpscSender(tx);
        let rx_wrapper = MonoioMpscReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }

    /// Accepts 1 to `tokio::sync::Semaphore::MAX_PERMITS`: the Tokio channel that this adapter
    /// uses panics on 0 and on a larger size.
    fn buffer_size_range() -> RangeInclusive<usize> {
        1..=tokio::sync::Semaphore::MAX_PERMITS
    }
}

impl<T> MpscSender<MonoioMpsc, T> for MonoioMpscSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> {
        self.0.send(msg).map_err(|e| SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <MonoioMpsc as Mpsc>::WeakSender<T> {
        let inner = self.0.downgrade();
        MonoioMpscWeakSender(inner)
    }
}

impl<T> MpscReceiver<T> for MonoioMpscReceiver<T> {
    #[inline]
    fn recv(&mut self) -> impl Future<Output = Option<T>> {
        self.0.recv()
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.0.try_recv().map_err(|e| match e {
            tokio_mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
            tokio_mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }
}

impl<T> MpscWeakSender<MonoioMpsc, T> for MonoioMpscWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<MonoioMpsc as Mpsc>::Sender<T>> {
        self.0.upgrade().map(MonoioMpscSender)
    }
}

#[cfg(test)]
mod tests {
    use openraft_rt::Mpsc;

    use super::MonoioMpsc;

    #[test]
    fn test_buffer_size_range_is_what_a_tokio_channel_accepts() {
        let range = MonoioMpsc::buffer_size_range();
        assert_eq!(1..=tokio::sync::Semaphore::MAX_PERMITS, range);

        let _channel = MonoioMpsc::channel::<u32>(*range.start());
        let _channel = MonoioMpsc::channel::<u32>(*range.end());

        let below = std::panic::catch_unwind(|| MonoioMpsc::channel::<u32>(range.start() - 1));
        assert!(below.is_err(), "a Tokio channel panics below the range");
        let above = std::panic::catch_unwind(|| MonoioMpsc::channel::<u32>(range.end() + 1));
        assert!(above.is_err(), "a Tokio channel panics above the range");
    }
}
