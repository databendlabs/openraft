use tokio::sync::watch as tokio_watch;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::async_runtime::watch;

pub struct TokioWatch;

impl watch::Watch for TokioWatch {
    type Sender<T: OptionalSend + OptionalSync> = tokio_watch::Sender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = tokio_watch::Receiver<T>;

    type Ref<'a, T: OptionalSend + 'a> = tokio_watch::Ref<'a, T>;

    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        tokio_watch::channel(init)
    }
}

impl<T> watch::WatchSender<TokioWatch, T> for tokio_watch::Sender<T>
where T: OptionalSend + OptionalSync
{
    fn send(&self, value: T) -> Result<(), watch::SendError<T>> {
        self.send(value).map_err(|e| watch::SendError(e.0))
    }

    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.send_if_modified(modify)
    }

    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        self.borrow()
    }

    fn subscribe(&self) -> <TokioWatch as watch::Watch>::Receiver<T> {
        self.subscribe()
    }
}

impl<T> watch::WatchReceiver<TokioWatch, T> for tokio_watch::Receiver<T>
where T: OptionalSend + OptionalSync
{
    async fn changed(&mut self) -> Result<(), watch::RecvError> {
        self.changed().await.map_err(|_| watch::RecvError(()))
    }

    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        self.borrow()
    }
}
