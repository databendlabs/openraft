use openraft::async_runtime::Mpsc;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::async_runtime::MpscWeakSender;
use openraft::async_runtime::SendError;
use openraft::async_runtime::TryRecvError;
use openraft::OptionalSend;

pub struct FlumeMpsc;

pub struct FlumeSender<T>(flume::Sender<T>);
pub struct FlumeWeakSender<T>(flume::WeakSender<T>);
pub struct FlumeReceiver<T>(flume::Receiver<T>);

impl<T> Clone for FlumeSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Clone for FlumeWeakSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Mpsc for FlumeMpsc {
    type Sender<T: OptionalSend> = FlumeSender<T>;
    type Receiver<T: OptionalSend> = FlumeReceiver<T>;
    type WeakSender<T: OptionalSend> = FlumeWeakSender<T>;

    #[inline]
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = flume::bounded(buffer);
        let tx_wrapper = FlumeSender(tx);
        let rx_wrapper = FlumeReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> MpscSender<FlumeMpsc, T> for FlumeSender<T>
where T: OptionalSend
{
    #[inline]
    async fn send(&self, msg: T) -> Result<(), SendError<T>> {
        self.0.send_async(msg).await.map_err(|e| SendError(e.into_inner()))
    }

    #[inline]
    fn downgrade(&self) -> <FlumeMpsc as Mpsc>::WeakSender<T> {
        FlumeWeakSender(self.0.downgrade())
    }
}

impl<T> MpscWeakSender<FlumeMpsc, T> for FlumeWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<FlumeMpsc as Mpsc>::Sender<T>> {
        self.0.upgrade().map(FlumeSender)
    }
}

impl<T> MpscReceiver<T> for FlumeReceiver<T> {
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
