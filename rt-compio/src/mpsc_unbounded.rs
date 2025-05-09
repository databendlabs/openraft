//! Unbounded MPSC channel wrapper types and their trait impl.

use openraft::type_config::async_runtime::mpsc_unbounded;
use openraft::OptionalSend;
use tokio::sync::mpsc as tokio_mpsc;

pub struct TokioMpscUnbounded;

pub struct TokioMpscUnboundedSender<T>(tokio_mpsc::UnboundedSender<T>);

impl<T> Clone for TokioMpscUnboundedSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct TokioMpscUnboundedReceiver<T>(tokio_mpsc::UnboundedReceiver<T>);

pub struct TokioMpscUnboundedWeakSender<T>(tokio_mpsc::WeakUnboundedSender<T>);

impl<T> Clone for TokioMpscUnboundedWeakSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl mpsc_unbounded::MpscUnbounded for TokioMpscUnbounded {
    type Sender<T: OptionalSend> = TokioMpscUnboundedSender<T>;
    type Receiver<T: OptionalSend> = TokioMpscUnboundedReceiver<T>;
    type WeakSender<T: OptionalSend> = TokioMpscUnboundedWeakSender<T>;

    #[inline]
    fn channel<T: OptionalSend>() -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = tokio_mpsc::unbounded_channel();
        let tx_wrapper = TokioMpscUnboundedSender(tx);
        let rx_wrapper = TokioMpscUnboundedReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> mpsc_unbounded::MpscUnboundedSender<TokioMpscUnbounded, T> for TokioMpscUnboundedSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> Result<(), mpsc_unbounded::SendError<T>> {
        self.0.send(msg).map_err(|e| mpsc_unbounded::SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <TokioMpscUnbounded as mpsc_unbounded::MpscUnbounded>::WeakSender<T> {
        let inner = self.0.downgrade();
        TokioMpscUnboundedWeakSender(inner)
    }
}

impl<T> mpsc_unbounded::MpscUnboundedReceiver<T> for TokioMpscUnboundedReceiver<T> {
    #[inline]
    async fn recv(&mut self) -> Option<T> {
        self.0.recv().await
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, mpsc_unbounded::TryRecvError> {
        self.0.try_recv().map_err(|e| match e {
            tokio_mpsc::error::TryRecvError::Empty => mpsc_unbounded::TryRecvError::Empty,
            tokio_mpsc::error::TryRecvError::Disconnected => mpsc_unbounded::TryRecvError::Disconnected,
        })
    }
}

impl<T> mpsc_unbounded::MpscUnboundedWeakSender<TokioMpscUnbounded, T> for TokioMpscUnboundedWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<TokioMpscUnbounded as mpsc_unbounded::MpscUnbounded>::Sender<T>> {
        self.0.upgrade().map(TokioMpscUnboundedSender)
    }
}
