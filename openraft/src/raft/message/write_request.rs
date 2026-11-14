use std::future::Future;
use std::future::IntoFuture;

use openraft_macros::since;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::errors::Fatal;
use crate::raft::message::WriteOption;
use crate::raft::message::write_request_args::WriteRequestArgs;
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
/// Awaiting the builder does not allocate the future on the heap. `F` is the concrete future
/// type returned when the builder is awaited; [`Raft::write()`] infers it. Code that names
/// the builder type takes `F` as a generic parameter:
///
/// ```ignore
/// fn submit<C, F>(request: WriteRequest<'_, C, F>)
/// where
///     C: RaftTypeConfig,
///     F: Future<Output = Result<(), Fatal<C>>> + OptionalSend,
/// {
///     // ...
/// }
/// ```
///
/// # Examples
///
/// ```ignore
/// // Fire-and-forget
/// raft.write(my_data).await?;
///
/// // With responder to receive result
/// let (responder, complete_rx) = ProgressResponder::complete_only();
/// raft.write(my_data).responder(responder).await?;
/// let result = complete_rx.await??;
/// ```
///
/// [`Raft::write()`]: crate::Raft::write
/// [`.responder()`]: WriteRequest::responder
#[since(
    version = "0.10.1",
    change = "add generic future parameter F to avoid boxing when awaited"
)]
#[since(version = "0.10.0")]
pub struct WriteRequest<'a, C, F>
where C: RaftTypeConfig
{
    /// Request arguments, still changeable by the builder methods.
    args: WriteRequestArgs<'a, C>,

    /// Turns `args` into the submission future; it is always `WriteRequestArgs::send`.
    ///
    /// A function pointer lets `F` be the anonymous future type of `send`, so the future does not
    /// need to be boxed.
    make_future: fn(WriteRequestArgs<'a, C>) -> F,
}

impl<C, F> WriteRequest<'_, C, F>
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
    /// let (responder, complete_rx) = ProgressResponder::complete_only();
    /// raft.write(my_data).responder(responder).await?;
    /// let result = complete_rx.await??;
    /// ```
    #[since(version = "0.10.0")]
    pub fn responder(mut self, responder: WriteResponderOf<C>) -> Self {
        self.args.responder = Some(CoreResponder::UserDefined(responder));
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
    /// [`ClientWriteError::ForwardToLeader`]: crate::errors::ClientWriteError::ForwardToLeader
    #[since(version = "0.10.0")]
    pub fn with_leader(mut self, expected_leader: impl Into<CommittedLeaderIdOf<C>>) -> Self {
        self.args.write_option = self.args.write_option.with_leader(expected_leader);
        self
    }

    /// Replace the [`WriteOption`] of this write, including an expected leader set by
    /// [`Self::with_leader()`].
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use openraft::raft::WriteOption;
    ///
    /// let term = raft.as_leader()?.term();
    /// raft.write(my_data)
    ///     .with_option(WriteOption::expect_leader(term))
    ///     .await?;
    /// ```
    #[since(version = "0.10.1")]
    pub fn with_option(mut self, write_option: WriteOption<C>) -> Self {
        self.args.write_option = write_option;
        self
    }
}

impl<C, F> IntoFuture for WriteRequest<'_, C, F>
where
    C: RaftTypeConfig,
    F: Future<Output = Result<(), Fatal<C>>>,
{
    type Output = Result<(), Fatal<C>>;
    type IntoFuture = F;

    fn into_future(self) -> Self::IntoFuture {
        (self.make_future)(self.args)
    }
}

/// Create the builder returned by [`Raft::write()`](crate::Raft::write).
///
/// The future type `F` is inferred from `WriteRequestArgs::send`.
pub(in crate::raft) fn write_request<C>(
    inner: &RaftInner<C>,
    app_data: C::D,
) -> WriteRequest<'_, C, impl Future<Output = Result<(), Fatal<C>>> + OptionalSend>
where
    C: RaftTypeConfig,
{
    WriteRequest {
        args: WriteRequestArgs {
            inner,
            app_data,
            responder: None,
            write_option: WriteOption::default(),
        },
        make_future: WriteRequestArgs::send,
    }
}
