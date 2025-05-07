//! Oneshot channel wrapper types and their trait impl.

use openraft::type_config::async_runtime::oneshot;
use openraft::OptionalSend;

pub struct FuturesOneshot;

pub struct FuturesOneshotSender<T>(futures::channel::oneshot::Sender<T>);

impl oneshot::Oneshot for FuturesOneshot {
    type Sender<T: OptionalSend> = FuturesOneshotSender<T>;
    type Receiver<T: OptionalSend> = futures::channel::oneshot::Receiver<T>;
    type ReceiverError = futures::channel::oneshot::Canceled;

    #[inline]
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend {
        let (tx, rx) = futures::channel::oneshot::channel();
        let tx_wrapper = FuturesOneshotSender(tx);

        (tx_wrapper, rx)
    }
}

impl<T> oneshot::OneshotSender<T> for FuturesOneshotSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.0.send(t)
    }
}
