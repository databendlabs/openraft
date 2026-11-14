use std::future::Future;
use std::future::IntoFuture;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::entry::RaftPayload;
use crate::errors::Fatal;
use crate::raft::message::WriteOption;
use crate::raft::message::write_many_request_args::WriteManyRequestArgs;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::WriteResponderOf;

/// Builder for submitting a batch of writes to Raft in one request.
///
/// Returned by [`Raft::write_many()`], which describes what RaftCore guarantees for the batch.
/// Each write is an application data and an optional responder that receives its result. Awaiting
/// the builder submits the batch and does not wait for the results.
///
/// `F` is the concrete future type returned when the builder is awaited, as in
/// [`WriteRequest`]; [`Raft::write_many()`] infers it.
///
/// # Examples
///
/// ```ignore
/// use openraft::impls::ProgressResponder;
///
/// let (responder, complete_rx) = ProgressResponder::complete_only();
/// raft.write_many([(data1, None)])
///     .push(data2, Some(responder))
///     .await?;
/// let result = complete_rx.await??;
/// ```
///
/// [`Raft::write_many()`]: crate::Raft::write_many
/// [`WriteRequest`]: crate::raft::WriteRequest
#[since(version = "0.10.1")]
pub struct WriteManyRequest<'a, C, F>
where C: RaftTypeConfig
{
    /// Request arguments, still changeable by the builder methods.
    pub(in crate::raft) args: WriteManyRequestArgs<'a, C>,

    /// Turns `args` into the submission future; it is always `WriteManyRequestArgs::send`.
    ///
    /// A function pointer lets `F` be the anonymous future type of `send`, so the future does not
    /// need to be boxed.
    pub(in crate::raft) make_future: fn(WriteManyRequestArgs<'a, C>) -> F,
}

impl<C, F> WriteManyRequest<'_, C, F>
where C: RaftTypeConfig
{
    /// Append a write after the writes already in the batch.
    ///
    /// `responder` receives the result of this write; with `None`, the result is not reported.
    #[since(version = "0.10.1")]
    pub fn push(mut self, app_data: C::D, responder: Option<WriteResponderOf<C>>) -> Self {
        self.append(app_data, responder);
        self
    }

    /// Append writes, in iteration order, after the writes already in the batch.
    ///
    /// Each item is an application data and an optional responder, as in [`Self::push()`].
    #[since(version = "0.10.1")]
    pub fn extend(mut self, writes: impl IntoIterator<Item = (C::D, Option<WriteResponderOf<C>>)>) -> Self {
        for (app_data, responder) in writes {
            self.append(app_data, responder);
        }
        self
    }

    /// Admit the batch only if the current committed leader is `expected_leader`.
    ///
    /// If it is not, every write in the batch fails with [`ClientWriteError::ForwardToLeader`] and
    /// nothing is appended. See [`WriteRequest::with_leader()`] for what a leader id contains
    /// under each leader id type.
    ///
    /// This sets the expected leader in the batch's [`WriteOption`].
    ///
    /// [`ClientWriteError::ForwardToLeader`]: crate::errors::ClientWriteError::ForwardToLeader
    /// [`WriteRequest::with_leader()`]: crate::raft::WriteRequest::with_leader
    #[since(version = "0.10.1")]
    pub fn with_leader(mut self, expected_leader: impl Into<CommittedLeaderIdOf<C>>) -> Self {
        self.args.write_option = self.args.write_option.with_leader(expected_leader);
        self
    }

    /// Replace the [`WriteOption`] of this batch, including an expected leader set by
    /// [`Self::with_leader()`].
    #[since(version = "0.10.1")]
    pub fn with_option(mut self, write_option: WriteOption<C>) -> Self {
        self.args.write_option = write_option;
        self
    }

    fn append(&mut self, app_data: C::D, responder: Option<WriteResponderOf<C>>) {
        self.args.payloads.extend([C::Payload::normal(app_data)]);
        self.args.responders.extend([responder.map(CoreResponder::UserDefined)]);
    }
}

impl<C, F> IntoFuture for WriteManyRequest<'_, C, F>
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
