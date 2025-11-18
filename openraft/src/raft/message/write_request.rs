use std::future::IntoFuture;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::core::raft_msg::RaftMsg;
use crate::error::Fatal;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::WriteResponderOf;

/// Builder for submitting write requests to Raft.
///
/// Returned by [`Raft::write()`]. The request is fire-and-forget by default.
/// Use [`.responder()`] to attach a responder for receiving the result.
///
/// # Performance
///
/// Currently, allocates (`Pin<Box<dyn Future>>`) when awaited due to stable Rust limitations.
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
    pub(in crate::raft) expected_leader: Option<CommittedLeaderIdOf<C>>,
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

    /// Execute write only if the current leader matches the expected leader.
    ///
    /// This method enables conditional writes that prevent operations from executing
    /// on a different leader than intended. Useful for preventing duplicate operations
    /// when retrying writes after network partitions or leadership changes.
    ///
    /// # Behavior
    ///
    /// - If the current committed leader matches `expected_leader`, the write proceeds normally.
    /// - If they don't match, returns [`ClientWriteError::ForwardToLeader`] without executing the
    ///   write.
    /// - The comparison is done before the write is appended to the log.
    ///
    /// **Note:** If leadership changed away from this node and then back, the error
    /// may still indicate this node as the leader, but with a different leader ID
    /// (e.g., different term).
    ///
    /// # Leader ID Types
    ///
    /// The type of `CommittedLeaderId` depends on your Raft configuration:
    ///
    /// - [`crate::impls::leader_id_std::LeaderId`] (standard Raft): `CommittedLeaderId` is just the
    ///   `term` number with in a transparent wrapper. Multiple elections in the same term are not
    ///   allowed.
    ///
    /// - [`crate::impls::leader_id_adv::LeaderId`] (advanced mode): `CommittedLeaderId` is the same
    ///   as `LeaderId`, similar to a tuple of `(term, node_id)`. This allows multiple leaders to be
    ///   elected in the same term (though not simultaneously).
    ///
    /// # Examples
    ///
    /// With standard LeaderId (term-only):
    /// ```ignore
    /// let term = raft.as_leader()?.term();
    /// raft.write(my_data)
    ///     .with_leader(term)
    ///     .await?;
    /// ```
    ///
    /// With advanced LeaderId (term + node_id):
    /// ```ignore
    /// let committed_leader_id = raft.as_leader().to_committed_leader_id();
    /// raft.write(my_data)
    ///     .with_leader(committed_leader_id)
    ///     .await?;
    /// ```
    ///
    /// See [`Leader::term()`] and [`Leader::to_committed_leader_id()`]
    ///
    /// [`ClientWriteError::ForwardToLeader`]: crate::error::ClientWriteError::ForwardToLeader
    #[since(version = "0.10.0")]
    pub fn with_leader(mut self, expected_leader: impl Into<CommittedLeaderIdOf<C>>) -> Self {
        self.expected_leader = Some(expected_leader.into());
        self
    }
}

impl<'a, C> IntoFuture for WriteRequest<'a, C>
where C: RaftTypeConfig
{
    type Output = Result<(), Fatal<C>>;
    type IntoFuture = BoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.inner
                .send_msg(RaftMsg::ClientWriteRequest {
                    app_data: self.app_data,
                    responder: self.responder,
                    expected_leader: self.expected_leader,
                })
                .await
        })
    }
}
