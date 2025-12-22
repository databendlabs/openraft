use crate::OptionalSend;
use crate::async_runtime::oneshot;
use crate::type_config::OneshotSender;

pub struct TokioOneshot;

impl oneshot::Oneshot for TokioOneshot {
    type Sender<T: OptionalSend> = tokio::sync::oneshot::Sender<T>;
    type Receiver<T: OptionalSend> = tokio::sync::oneshot::Receiver<T>;
    type ReceiverError = tokio::sync::oneshot::error::RecvError;

    #[inline]
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (tx, rx)
    }
}

impl<T> OneshotSender<T> for tokio::sync::oneshot::Sender<T>
where T: OptionalSend
{
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.send(t)
    }
}
