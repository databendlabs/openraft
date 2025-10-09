use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::raft::message::ClientWriteResult;
use crate::raft::responder::Responder;
use crate::raft::responder::ResponderBuilder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;

/// A [`Responder`] implementation that sends the response via a oneshot channel.
///
/// This could be used when the [`Raft::client_write`] caller wants to wait for the response.
///
/// [`Raft::client_write`]: `crate::raft::Raft::client_write`
pub struct OneshotResponder<C>
where C: RaftTypeConfig
{
    tx: OneshotSenderOf<C, ClientWriteResult<C>>,
}

impl<C> OneshotResponder<C>
where C: RaftTypeConfig
{
    /// Create a new instance from a [`AsyncRuntime::Oneshot::Sender`].
    ///
    /// [`AsyncRuntime::Oneshot::Sender`]: `crate::async_runtime::Oneshot::Sender`
    pub fn new(tx: OneshotSenderOf<C, ClientWriteResult<C>>) -> Self {
        Self { tx }
    }
}

impl<C> Responder<ClientWriteResult<C>> for OneshotResponder<C>
where C: RaftTypeConfig
{
    fn send(self, res: ClientWriteResult<C>) {
        let res = self.tx.send(res);

        if res.is_ok() {
            tracing::debug!("OneshotConsumer.tx.send: is_ok: {}", res.is_ok());
        } else {
            tracing::warn!("OneshotConsumer.tx.send: is_ok: {}", res.is_ok());
        }
    }
}

impl<C> ResponderBuilder<C::D, ClientWriteResult<C>> for OneshotResponder<C>
where C: RaftTypeConfig
{
    type Responder = Self;
    type Receiver = OneshotReceiverOf<C, ClientWriteResult<C>>;

    fn build(_src: &C::D) -> (Self::Responder, Self::Receiver) {
        let (tx, rx) = C::oneshot();
        (Self { tx }, rx)
    }
}
