use openraft_macros::since;

use crate::AsyncRuntime;
use crate::Oneshot;

/// Signals cancellation of a future created by [`AsyncRuntime::cancellable`].
///
/// Dropping the handle also signals cancellation. The caller drives the wrapped future.
#[since(version = "0.10.1")]
pub struct CancelHandle<R>
where R: AsyncRuntime
{
    /// The wrapped future waits on the receiving end, which completes once this sender is
    /// dropped, so dropping it is the cancel signal.
    cancel: Option<<R::Oneshot as Oneshot>::Sender<()>>,
}

impl<R> CancelHandle<R>
where R: AsyncRuntime
{
    /// Create a handle and the receiver that completes when the handle cancels or drops.
    pub(crate) fn new() -> (Self, <R::Oneshot as Oneshot>::Receiver<()>) {
        let (cancel, cancelled) = R::Oneshot::channel();
        let handle = Self { cancel: Some(cancel) };
        (handle, cancelled)
    }

    /// Signal cancellation once. Cancelling an already completed future is harmless.
    #[since(version = "0.10.1")]
    pub fn cancel(&mut self) {
        self.cancel = None;
    }
}
