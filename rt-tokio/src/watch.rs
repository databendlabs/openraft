use tokio::sync::watch as tokio_watch;

use openraft_rt::watch;
use openraft_rt::OptionalSend;
use openraft_rt::OptionalSync;

pub struct TokioWatch;

/// Wrapper around `tokio::sync::watch::Sender` to implement the `WatchSender` trait.
pub struct TokioWatchSender<T>(tokio_watch::Sender<T>);

impl<T> Clone for TokioWatchSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Wrapper around `tokio::sync::watch::Receiver` to implement the `WatchReceiver` trait.
pub struct TokioWatchReceiver<T>(tokio_watch::Receiver<T>);

impl<T> Clone for TokioWatchReceiver<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl watch::Watch for TokioWatch {
    type Sender<T: OptionalSend + OptionalSync> = TokioWatchSender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = TokioWatchReceiver<T>;

    type Ref<'a, T: OptionalSend + 'a> = tokio_watch::Ref<'a, T>;

    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = tokio_watch::channel(init);
        (TokioWatchSender(tx), TokioWatchReceiver(rx))
    }
}

impl<T> watch::WatchSender<TokioWatch, T> for TokioWatchSender<T>
where T: OptionalSend + OptionalSync
{
    fn send(&self, value: T) -> Result<(), watch::SendError<T>> {
        self.0.send(value).map_err(|e| watch::SendError(e.0))
    }

    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.0.send_if_modified(modify)
    }

    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        self.0.borrow()
    }

    fn subscribe(&self) -> <TokioWatch as watch::Watch>::Receiver<T> {
        TokioWatchReceiver(self.0.subscribe())
    }
}

impl<T> watch::WatchReceiver<TokioWatch, T> for TokioWatchReceiver<T>
where T: OptionalSend + OptionalSync
{
    async fn changed(&mut self) -> Result<(), watch::RecvError> {
        self.0.changed().await.map_err(|_| watch::RecvError(()))
    }

    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        self.0.borrow()
    }
}
