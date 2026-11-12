use openraft_macros::since;

use crate::AsyncRuntime;
use crate::Oneshot;
use crate::OneshotSender;

/// Signals cancellation of a future created by [`AsyncRuntime::cancellable`].
///
/// Dropping the handle also signals cancellation. The caller drives the wrapped future.
#[since(version = "0.10.1")]
pub struct CancelHandle<R>
where R: AsyncRuntime
{
    pub(crate) cancel: Option<<R::Oneshot as Oneshot>::Sender<()>>,
}

impl<R> CancelHandle<R>
where R: AsyncRuntime
{
    /// Signal cancellation once. Cancelling an already completed future is harmless.
    #[since(version = "0.10.1")]
    pub fn cancel(&mut self) {
        let Some(cancel) = self.cancel.take() else {
            return;
        };
        let _ = cancel.send(());
    }
}

impl<R> Drop for CancelHandle<R>
where R: AsyncRuntime
{
    fn drop(&mut self) {
        self.cancel();
    }
}
