use std::future::Future;
use std::future::IntoFuture;
use std::pin::Pin;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::core::raft_msg::RaftMsg;
use crate::error::Fatal;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::alias::WriteResponderOf;

/// Builder for submitting write requests to Raft.
///
/// Returned by [`Raft::write()`]. The request is fire-and-forget by default.
/// Use [`.responder()`] to attach a responder for receiving the result.
///
/// # Performance
///
/// Currently allocates (`Pin<Box<dyn Future>>`) when awaited due to stable Rust limitations.
/// This will be optimized to zero-allocation when `impl_trait_in_assoc_type` stabilizes,
/// allowing `type IntoFuture = impl Future` in trait implementations.
///
/// # Examples
///
/// ```ignore
/// // Fire-and-forget
/// raft.write(my_data).await?;
///
/// // With responder to receive result
/// let (responder, _commit_rx, complete_rx) = ProgressResponder::new();
/// raft.write(my_data).responder(responder).await?;
/// let result = complete_rx.await??;
/// ```
///
/// [`Raft::write()`]: crate::Raft::write
/// [`.responder()`]: WriteRequest::responder
#[since(version = "0.10.0")]
pub struct WriteRequest<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) inner: &'a RaftInner<C>,
    pub(in crate::raft) app_data: C::D,
    pub(in crate::raft) responder: Option<CoreResponder<C>>,
}

impl<'a, C> WriteRequest<'a, C>
where C: RaftTypeConfig
{
    /// Attach a responder to receive the write result.
    ///
    /// The responder is notified when the write completes or fails.
    /// Await the responder's receiver to get the result.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use openraft::impls::ProgressResponder;
    ///
    /// let (responder, _commit_rx, complete_rx) = ProgressResponder::new();
    /// raft.write(my_data).responder(responder).await?;
    /// let result = complete_rx.await??;
    /// ```
    #[since(version = "0.10.0")]
    pub fn responder(mut self, responder: WriteResponderOf<C>) -> Self {
        self.responder = Some(CoreResponder::UserDefined(responder));
        self
    }
}

impl<'a, C> IntoFuture for WriteRequest<'a, C>
where C: RaftTypeConfig
{
    type Output = Result<(), Fatal<C>>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.inner
                .send_msg(RaftMsg::ClientWriteRequest {
                    app_data: self.app_data,
                    responder: self.responder,
                })
                .await
        })
    }
}
