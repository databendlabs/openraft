//! One-shot channels, wrapping `tokio::sync::oneshot`.

use openraft_rt::OneshotSender;
use openraft_rt::OptionalSend;
use openraft_rt::oneshot;

/// One-shot channel.
pub struct SimOneshot;

/// Wrapper around `tokio::sync::oneshot::Sender` to implement the `OneshotSender` trait.
pub struct SimOneshotSender<T>(tokio::sync::oneshot::Sender<T>);

impl oneshot::Oneshot for SimOneshot {
    type Sender<T: OptionalSend> = SimOneshotSender<T>;
    type Receiver<T: OptionalSend> = tokio::sync::oneshot::Receiver<T>;
    type ReceiverError = tokio::sync::oneshot::error::RecvError;

    #[inline]
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (SimOneshotSender(tx), rx)
    }
}

impl<T> OneshotSender<T> for SimOneshotSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.0.send(t)
    }
}
