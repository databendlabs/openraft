use std::ops::Deref;

use openraft::async_runtime::watch::RecvError;
use openraft::async_runtime::watch::SendError;
use openraft::type_config::async_runtime::watch;
use openraft::OptionalSend;
use openraft::OptionalSync;
use tokio::sync::watch as tokio_watch;

pub struct TokioWatch;
pub struct TokioWatchSender<T>(tokio_watch::Sender<T>);
pub struct TokioWatchReceiver<T>(tokio_watch::Receiver<T>);
pub struct TokioWatchRef<'a, T>(tokio_watch::Ref<'a, T>);

impl watch::Watch for TokioWatch {
    type Sender<T: OptionalSend + OptionalSync> = TokioWatchSender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = TokioWatchReceiver<T>;
    type Ref<'a, T: OptionalSend + 'a> = TokioWatchRef<'a, T>;

    #[inline]
    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = tokio_watch::channel(init);
        let tx_wrapper = TokioWatchSender(tx);
        let rx_wrapper = TokioWatchReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> watch::WatchSender<TokioWatch, T> for TokioWatchSender<T>
where T: OptionalSend + OptionalSync
{
    #[inline]
    fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.send(value).map_err(|e| watch::SendError(e.0))
    }

    #[inline]
    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.0.send_if_modified(modify)
    }

    #[inline]
    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        let inner = self.0.borrow();
        TokioWatchRef(inner)
    }
}

impl<T> Clone for TokioWatchReceiver<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> watch::WatchReceiver<TokioWatch, T> for TokioWatchReceiver<T>
where T: OptionalSend + OptionalSync
{
    #[inline]
    async fn changed(&mut self) -> Result<(), RecvError> {
        self.0.changed().await.map_err(|_| watch::RecvError(()))
    }

    #[inline]
    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        TokioWatchRef(self.0.borrow())
    }
}

impl<T> Deref for TokioWatchRef<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}
