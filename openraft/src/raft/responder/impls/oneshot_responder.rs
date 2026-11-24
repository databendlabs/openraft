use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::raft::responder::Responder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;

/// A [`Responder`] that sends the result of a write through a oneshot channel of the configured
/// [`AsyncRuntime`].
///
/// It sends only the final result and ignores the commit notification. Use
/// [`ProgressResponder`] to also be notified when the write is committed.
///
/// # Usage
///
/// Select this type as `Responder<T>` in the type config, as the example below does: the write
/// APIs that take a responder accept only the configured type, and [`declare_raft_types!`]
/// selects [`ProgressResponder`] by default. Then create a responder with
/// [`new_pair()`](Self::new_pair), keep the receiver, and attach the responder to a write with
/// [`WriteRequest::responder()`], [`Raft::write_many()`] or [`Raft::client_write_ff()`].
///
/// APIs that return the result of a write themselves, such as [`Raft::client_write()`],
/// [`Raft::client_write_many()`] and [`Raft::change_membership()`], wait on a responder that
/// they create internally. They do not use the configured responder type, and selecting
/// `OneshotResponder` does not change them.
///
/// # Example
///
/// ```no_run
/// # use openraft::Raft;
/// # use openraft::storage::RaftStateMachine;
/// use openraft::impls::OneshotResponder;
///
/// openraft::declare_raft_types!(
///     TypeConfig:
///         Responder<T> = OneshotResponder<Self, T>,
/// );
///
/// # async fn example<SM>(
/// #     raft: &Raft<TypeConfig, SM>,
/// #     request: String,
/// # ) -> Result<(), Box<dyn std::error::Error>>
/// # where SM: RaftStateMachine<TypeConfig>
/// # {
/// let (responder, rx) = OneshotResponder::new_pair();
///
/// // `Err` means that RaftCore has stopped and did not receive the write. `Ok` means only that
/// // the write was submitted: RaftCore may still reject it.
/// raft.write(request).responder(responder).await?;
///
/// let result = match rx.await {
///     Ok(result) => result,
///     // The responder was dropped without a result, such as when RaftCore stopped.
///     // The write may still be committed and applied.
///     Err(_) => return Err("no result arrived for the write".into()),
/// };
///
/// // The result arrived: the applied write's response, or the write's error, such as
/// // `ForwardToLeader` from a follower.
/// let response = result?;
/// println!("applied at {}: {}", response.log_id, response.data);
/// # Ok(())
/// # }
/// ```
///
/// [`AsyncRuntime`]: crate::AsyncRuntime
/// [`ProgressResponder`]: crate::impls::ProgressResponder
/// [`declare_raft_types!`]: crate::declare_raft_types
/// [`WriteRequest::responder()`]: crate::raft::WriteRequest::responder
/// [`Raft::write_many()`]: crate::Raft::write_many
/// [`Raft::client_write_ff()`]: crate::Raft::client_write_ff
/// [`Raft::client_write()`]: crate::Raft::client_write
/// [`Raft::client_write_many()`]: crate::Raft::client_write_many
/// [`Raft::change_membership()`]: crate::Raft::change_membership
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
            tracing::debug!("OneshotResponder.tx.send: is_ok: {}", res.is_ok());
        } else {
            tracing::warn!("OneshotResponder.tx.send: is_ok: {}", res.is_ok());
        }
    }
}
