use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::raft::responder::Responder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;

/// A [`Responder`] implementation that sends the response via a oneshot channel.
///
/// This could be used when the [`Raft::client_write`] caller wants to wait for the response.
///
/// [`Raft::client_write`]: `crate::raft::Raft::client_write`
pub struct OneshotResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    tx: OneshotSenderOf<C, T>,
}

impl<C, T> OneshotResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    /// Create a new instance from a [`AsyncRuntime::Oneshot::Sender`].
    ///
    /// [`AsyncRuntime::Oneshot::Sender`]: `crate::async_runtime::Oneshot::Sender`
    pub fn new(tx: OneshotSenderOf<C, T>) -> Self {
        Self { tx }
    }

    /// Create a new responder and receiver pair.
    ///
    /// This is a convenience method that creates a oneshot channel and returns
    /// a [`OneshotResponder`] wrapping the sender and the receiver.
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - The [`OneshotResponder`] that can be used to send a response
    /// - The receiver that can be used to wait for the response
    pub fn new_pair() -> (Self, OneshotReceiverOf<C, T>) {
        let (tx, rx) = C::oneshot();
        (Self::new(tx), rx)
    }
}

impl<C, T> Responder<C, T> for OneshotResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + 'static,
{
    fn on_complete(self, res: T) {
        let res = self.tx.send(res);

        if res.is_ok() {
            tracing::debug!("OneshotConsumer.tx.send: is_ok: {}", res.is_ok());
        } else {
            tracing::warn!("OneshotConsumer.tx.send: is_ok: {}", res.is_ok());
        }
    }
}
