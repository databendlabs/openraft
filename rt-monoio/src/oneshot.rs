//! Oneshot channel wrapper types and their trait impl.

use local_sync::oneshot as monoio_oneshot;
use openraft::type_config::async_runtime::oneshot;
use openraft::OptionalSend;

pub struct MonoioOneshot;

pub struct MonoioOneshotSender<T>(monoio_oneshot::Sender<T>);

impl oneshot::Oneshot for MonoioOneshot {
    type Sender<T: OptionalSend> = MonoioOneshotSender<T>;
    type Receiver<T: OptionalSend> = monoio_oneshot::Receiver<T>;
    type ReceiverError = monoio_oneshot::error::RecvError;

    #[inline]
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend {
        let (tx, rx) = monoio_oneshot::channel();
        let tx_wrapper = MonoioOneshotSender(tx);

        (tx_wrapper, rx)
    }
}

impl<T> oneshot::OneshotSender<T> for MonoioOneshotSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.0.send(t)
    }
}
