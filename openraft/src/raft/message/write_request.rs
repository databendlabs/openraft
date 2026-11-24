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
/// Returned by [`Raft::write()`]. Awaiting the builder submits the write to RaftCore and returns
/// without waiting for RaftCore to handle it:
///
/// - `Ok(())` means only that the write was submitted. RaftCore may still reject it, and a write
///   that RaftCore admits may never be committed or applied.
/// - `Err(Fatal)` means RaftCore has stopped and the write was not submitted.
///
/// The write is fire-and-forget by default: nothing reports whether it was rejected, committed, or
/// applied. Use [`.responder()`] to receive its result.
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
#[must_use = "write requests must be awaited to submit them"]
#[since(version = "0.10.2", change = "marked #[must_use]")]
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
    /// RaftCore completes the responder with the write's [`ClientWriteResult`]:
    ///
    /// - `Ok` with a [`ClientWriteResponse`] once the state machine has applied the write. It holds
    ///   the log id of the write and the response of the state machine.
    /// - `Err` with a [`ClientWriteError`] if the write fails, such as
    ///   [`ClientWriteError::ForwardToLeader`] when RaftCore rejects it without appending it, or
    ///   [`ClientWriteError::LogEntryDiscarded`] when its appended entry is discarded and the write
    ///   may still commit.
    ///
    /// If the responder is dropped without a result, for example because RaftCore stopped, the
    /// receiver of a [`ProgressResponder`] returns an error that says the channel closed. That
    /// error does not tell whether the write was committed or applied.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use openraft::impls::ProgressResponder;
    ///
    /// let (responder, complete_rx) = ProgressResponder::complete_only();
    /// raft.write(my_data).responder(responder).await?;
    /// // The first `?` returns the error of a closed channel, the second a `ClientWriteError`.
    /// let result = complete_rx.await??;
    /// ```
    ///
    /// [`ClientWriteResult`]: crate::raft::ClientWriteResult
    /// [`ClientWriteResponse`]: crate::raft::ClientWriteResponse
    /// [`ClientWriteError`]: crate::errors::ClientWriteError
    /// [`ClientWriteError::ForwardToLeader`]: crate::errors::ClientWriteError::ForwardToLeader
    /// [`ClientWriteError::LogEntryDiscarded`]: crate::errors::ClientWriteError::LogEntryDiscarded
    /// [`ProgressResponder`]: crate::impls::ProgressResponder
    #[since(version = "0.10.0")]
    pub fn responder(mut self, responder: WriteResponderOf<C>) -> Self {
        self.args.responder = Some(CoreResponder::UserDefined(responder));
        self
    }

    /// Admit the write only if the current committed leader is `expected_leader`.
    ///
    /// # Behavior
    ///
    /// RaftCore compares its current committed leader with `expected_leader` before it appends the
    /// write:
    ///
    /// - If they match, the write proceeds as it would without the condition.
    /// - If they don't match, RaftCore does not append the write and completes its responder with
    ///   [`ClientWriteError::ForwardToLeader`].
    ///
    /// Awaiting the builder returns `Ok(())` in both cases, because RaftCore compares the leaders
    /// after the write is submitted. Only an attached [responder](Self::responder) tells the cases
    /// apart; without one, a rejected write is dropped silently.
    ///
    /// The [`ForwardToLeader::leader_id`] of the error is a node id. If leadership moved away from
    /// this node and back, it names this node again, and the error does not carry the new term or
    /// committed leader id.
    ///
    /// # Retries
    ///
    /// The condition restricts which leader may append the write; it does not deduplicate writes. A
    /// retry under the same leader passes the condition again and is appended as a new entry, so if
    /// an earlier attempt committed, the state machine receives the write twice. Expecting a new
    /// leader for the retry does not settle the earlier attempt either: it may have committed under
    /// the old leader, or may still commit through the new one.
    ///
    /// To retry a write whose outcome is unknown, such as after a lost response or
    /// [`ClientWriteError::LogEntryDiscarded`], the state machine must deduplicate it, as described
    /// in [`Raft::client_write()`].
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
    /// ```no_run
    /// # use openraft::Raft;
    /// # use openraft::errors::ClientWriteError;
    /// # use openraft::impls::ProgressResponder;
    /// # use openraft::storage::RaftStateMachine;
    /// # openraft::declare_raft_types!(
    /// #     TypeConfig:
    /// #         LeaderId = openraft::impls::leader_id_std::LeaderId<Self::Term, Self::NodeId>,
    /// # );
    /// # async fn example<SM>(
    /// #     raft: &Raft<TypeConfig, SM>,
    /// #     my_data: String,
    /// # ) -> Result<(), Box<dyn std::error::Error>>
    /// # where SM: RaftStateMachine<TypeConfig> {
    /// let term = raft.as_leader()?.term();
    ///
    /// let (responder, complete_rx) = ProgressResponder::complete_only();
    /// // Returns `Ok` even if the leader has changed: RaftCore checks the leader later.
    /// raft.write(my_data).with_leader(term).responder(responder).await?;
    ///
    /// // `?` returns the error of a closed channel: no result arrived.
    /// match complete_rx.await? {
    ///     Ok(response) => println!("applied at {}", response.log_id()),
    ///     // Not appended, for example because the leader has changed.
    ///     Err(ClientWriteError::ForwardToLeader(forward)) => println!("rejected: {forward}"),
    ///     // Such as `LogEntryDiscarded`, after which the write may still commit.
    ///     Err(other) => return Err(other.into()),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// With advanced LeaderId (term + node_id):
    /// ```no_run
    /// # use openraft::Raft;
    /// # use openraft::impls::ProgressResponder;
    /// # use openraft::storage::RaftStateMachine;
    /// # openraft::declare_raft_types!(
    /// #     TypeConfig:
    /// #         LeaderId = openraft::impls::leader_id_adv::LeaderId<Self::Term, Self::NodeId>,
    /// # );
    /// # async fn example<SM>(
    /// #     raft: &Raft<TypeConfig, SM>,
    /// #     my_data: String,
    /// # ) -> Result<(), Box<dyn std::error::Error>>
    /// # where SM: RaftStateMachine<TypeConfig> {
    /// let committed_leader_id = raft.as_leader()?.to_committed_leader_id();
    ///
    /// let (responder, complete_rx) = ProgressResponder::complete_only();
    /// raft.write(my_data).with_leader(committed_leader_id).responder(responder).await?;
    ///
    /// // The first `?` returns the error of a closed channel, the second a `ClientWriteError`,
    /// // such as `ForwardToLeader` if the leader has changed.
    /// let response = complete_rx.await??;
    /// println!("applied at {}", response.log_id());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`Leader::term()`] and [`Leader::to_committed_leader_id()`]
    ///
    /// [`ClientWriteError::ForwardToLeader`]: crate::errors::ClientWriteError::ForwardToLeader
    /// [`ClientWriteError::LogEntryDiscarded`]: crate::errors::ClientWriteError::LogEntryDiscarded
    /// [`ForwardToLeader::leader_id`]: crate::errors::ForwardToLeader::leader_id
    /// [`Leader::term()`]: crate::raft::Leader::term
    /// [`Leader::to_committed_leader_id()`]: crate::raft::Leader::to_committed_leader_id
    /// [`Raft::client_write()`]: crate::Raft::client_write
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
    /// ```no_run
    /// # use openraft::Raft;
    /// # use openraft::RaftTypeConfig;
    /// # use openraft::storage::RaftStateMachine;
    /// use openraft::raft::WriteOption;
    ///
    /// # async fn example<C, SM>(
    /// #     raft: &Raft<C, SM>,
    /// #     my_data: C::D,
    /// # ) -> Result<(), Box<dyn std::error::Error>>
    /// # where
    /// #     C: RaftTypeConfig,
    /// #     SM: RaftStateMachine<C>,
    /// # {
    /// // Admit the write only while the leader that `as_leader()` returned still leads.
    /// let leader_id = raft.as_leader()?.to_committed_leader_id();
    /// raft.write(my_data)
    ///     .with_option(WriteOption::expect_leader(leader_id))
    ///     .await?;
    /// # Ok(())
    /// # }
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
