mod watch_error;

use openraft_macros::add_async_trait;
pub use watch_error::RecvError;
pub use watch_error::SendError;

use crate::OptionalSend;
use crate::OptionalSync;

pub trait Watch: Sized + OptionalSend {
    type Sender<T: OptionalSend + OptionalSync>: WatchSender<Self, T>;
    type Receiver<T: OptionalSend + OptionalSync>: WatchReceiver<Self, T>;

    type Ref<'a, T: OptionalSend + 'a>: std::ops::Deref<Target = T> + 'a;

    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>);
}

pub trait WatchSender<W, T>: OptionalSend + Clone
where
    W: Watch,
    T: OptionalSend + OptionalSync,
{
    fn send(&self, value: T) -> Result<(), SendError<T>>;
    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool;

    fn borrow_watched(&self) -> W::Ref<'_, T>;
}

#[add_async_trait]
pub trait WatchReceiver<W, T>: OptionalSend + OptionalSync + Clone
where
    W: Watch,
    T: OptionalSend + OptionalSync,
{
    async fn changed(&mut self) -> Result<(), RecvError>;
    fn borrow_watched(&self) -> W::Ref<'_, T>;
}
