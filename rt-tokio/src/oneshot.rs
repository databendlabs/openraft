use openraft_rt::OneshotSender;
use openraft_rt::OptionalSend;
use openraft_rt::oneshot;

pub struct TokioOneshot;

/// Wrapper around `tokio::sync::oneshot::Sender` to implement the `OneshotSender` trait.
pub struct TokioOneshotSender<T>(tokio::sync::oneshot::Sender<T>);

impl oneshot::Oneshot for TokioOneshot {
    type Sender<T: OptionalSend> = TokioOneshotSender<T>;
    type Receiver<T: OptionalSend> = tokio::sync::oneshot::Receiver<T>;
    type ReceiverError = tokio::sync::oneshot::error::RecvError;

    #[inline]
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (TokioOneshotSender(tx), rx)
    }
}

impl<T> OneshotSender<T> for TokioOneshotSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.0.send(t)
    }
}
