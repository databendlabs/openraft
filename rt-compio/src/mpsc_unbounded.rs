//! Unbounded MPSC channel wrapper types and their trait impl.
use openraft::async_runtime::MpscUnboundedReceiver;
use openraft::async_runtime::MpscUnboundedSender;
use openraft::async_runtime::MpscUnboundedWeakSender;
use openraft::async_runtime::SendError;
use openraft::async_runtime::TryRecvError;
use openraft::type_config::MpscUnbounded;
use openraft::OptionalSend;

pub struct FlumeMpscUnbounded;

pub struct FlumeUnboundedSender<T>(flume::Sender<T>);
pub struct FlumeUnboundedWeakSender<T>(flume::WeakSender<T>);
pub struct FlumeUnboundedReceiver<T>(flume::Receiver<T>);

impl<T> Clone for FlumeUnboundedSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Clone for FlumeUnboundedWeakSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl MpscUnbounded for FlumeMpscUnbounded {
    type Sender<T: OptionalSend> = FlumeUnboundedSender<T>;
    type Receiver<T: OptionalSend> = FlumeUnboundedReceiver<T>;
    type WeakSender<T: OptionalSend> = FlumeUnboundedWeakSender<T>;

    #[inline]
    fn channel<T: OptionalSend>() -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = flume::unbounded();
        let tx_wrapper = FlumeUnboundedSender(tx);
        let rx_wrapper = FlumeUnboundedReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> MpscUnboundedSender<FlumeMpscUnbounded, T> for FlumeUnboundedSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> Result<(), SendError<T>> {
        self.0.send(msg).map_err(|e| SendError(e.into_inner()))
    }

    #[inline]
    fn downgrade(&self) -> <FlumeMpscUnbounded as MpscUnbounded>::WeakSender<T> {
        FlumeUnboundedWeakSender(self.0.downgrade())
    }
}

impl<T> MpscUnboundedWeakSender<FlumeMpscUnbounded, T> for FlumeUnboundedWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<FlumeMpscUnbounded as MpscUnbounded>::Sender<T>> {
        self.0.upgrade().map(FlumeUnboundedSender)
    }
}

impl<T> MpscUnboundedReceiver<T> for FlumeUnboundedReceiver<T> {
    #[inline]
    async fn recv(&mut self) -> Option<T> {
        self.0.recv_async().await.ok()
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.0.try_recv().map_err(|e| match e {
            flume::TryRecvError::Empty => TryRecvError::Empty,
            flume::TryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }
}
