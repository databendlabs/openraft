use crate::async_runtime::AsyncOneshotSendExt;
use crate::raft::message::WriteResult;
use crate::raft::responder::Responder;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::AsyncRuntime;
use crate::RaftTypeConfig;

/// A [`Responder`] implementation that sends the response via a oneshot channel.
///
/// This could be used when the [`Raft::client_write`] caller want to wait for the response.
///
/// [`Raft::client_write`]: `crate::raft::Raft::client_write`
pub struct OneshotResponder<C>
where C: RaftTypeConfig
{
    tx: OneshotSenderOf<C, WriteResult<C>>,
}

impl<C> OneshotResponder<C>
where C: RaftTypeConfig
{
    /// Create a new instance from a [`AsyncRuntime::OneshotSender`].
    pub fn new(tx: OneshotSenderOf<C, WriteResult<C>>) -> Self {
        Self { tx }
    }
}

impl<C> Responder<C> for OneshotResponder<C>
where C: RaftTypeConfig
{
    type Receiver = OneshotReceiverOf<C, WriteResult<C>>;

    fn from_app_data(app_data: C::D) -> (C::D, Self, Self::Receiver)
    where Self: Sized {
        let (tx, rx) = AsyncRuntimeOf::<C>::oneshot();
        (app_data, Self { tx }, rx)
    }

    fn send(self, res: WriteResult<C>) {
        let res = self.tx.send(res);

        if res.is_ok() {
            tracing::debug!("OneshotConsumer.tx.send: is_ok: {}", res.is_ok());
        } else {
            tracing::warn!("OneshotConsumer.tx.send: is_ok: {}", res.is_ok());
        }
    }
}
