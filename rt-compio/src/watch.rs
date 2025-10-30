use std::ops::Deref;

use openraft::async_runtime::watch::RecvError;
use openraft::async_runtime::watch::SendError;
use openraft::type_config::async_runtime::watch;
use openraft::OptionalSend;
use openraft::OptionalSync;
use see::error::SendError as SeeSendError;
use see::unsync as see_unsync;

pub struct See;
pub struct SeeSender<T>(see_unsync::Sender<T>);
pub struct SeeReceiver<T>(see_unsync::Receiver<T>);
pub struct SeeRef<'a, T>(see_unsync::Guard<'a, T>);

impl watch::Watch for See {
    type Sender<T: OptionalSend + OptionalSync> = SeeSender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = SeeReceiver<T>;
    type Ref<'a, T: OptionalSend + 'a> = SeeRef<'a, T>;

    #[inline]
    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = see_unsync::channel(init);
        let tx_wrapper = SeeSender(tx);
        let rx_wrapper = SeeReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> watch::WatchSender<See, T> for SeeSender<T>
where T: OptionalSend + OptionalSync
{
    #[inline]
    fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.send(value).map_err(|e| match e {
            SeeSendError::ChannelClosed(value) => watch::SendError(value),
        })
    }

    #[inline]
    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.0.send_if_modified(modify)
    }

    #[inline]
    fn borrow_watched(&self) -> <See as watch::Watch>::Ref<'_, T> {
        let inner = self.0.borrow();
        SeeRef(inner)
    }
}

impl<T> Clone for SeeReceiver<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> watch::WatchReceiver<See, T> for SeeReceiver<T>
where T: OptionalSend + OptionalSync
{
    #[inline]
    async fn changed(&mut self) -> Result<(), RecvError> {
        self.0.changed().await.map_err(|_| watch::RecvError(()))
    }

    #[inline]
    fn borrow_watched(&self) -> <See as watch::Watch>::Ref<'_, T> {
        SeeRef(self.0.borrow())
    }
}

impl<T> Deref for SeeRef<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}
