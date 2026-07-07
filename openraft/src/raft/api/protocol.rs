use std::sync::Arc;
use std::time::Duration;

use display_more::DisplayOptionExt;
use futures_util::Stream;
use openraft_macros::since;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscWeakSender;
use crate::core::io_flush_tracking::FlushPoint;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::install_full_snapshot_request::InstallFullSnapshotRequest;
use crate::core::sm;
use crate::errors::Fatal;
#[cfg(doc)]
use crate::errors::into_raft_result::IntoRaftResult;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::TransferLeaderError;
use crate::raft::TransferLeaderRequest;
use crate::raft::TransferLeaderResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::raft_inner::RaftInner;
use crate::raft::stream_append;
use crate::raft::stream_append::StreamAppendResult;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MpscWeakSenderOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// Handles Raft protocol requests from other nodes in the cluster.
///
/// Usage: [`Raft::protocol_api()`](crate::raft::Raft::protocol_api)
///
/// This API uses a nested `Result<Result<_,E>,Fatal>` pattern to separate different error types:
/// - The outer `Result` contains [`Fatal`] errors that indicate the Raft node itself has failed
/// - The inner `Result` contains application-specific errors that should be handled or forwarded
///
/// This separation allows application code to handle fatal errors (typically by terminating or
/// restarting the node) separately from regular application errors that can be transmitted back
/// to RPC callers. In most cases, you'll use the `?` operator on the outer result and then
/// handle the inner result appropriately.
///
/// To convert the nested `Result<Result<_,E>,Fatal>` to a single `Result<_,RaftError<C,E>>`,
/// use the [`into_raft_result()`](IntoRaftResult::into_raft_result) method.
///
/// This struct provides methods for processing core Raft protocol messages such as
/// - Vote requests (RequestVote RPCs)
/// - AppendEntries RPCs (for log replication and heartbeats)
/// - Snapshot-related operations
///
/// It acts as an interface between the service layer and the internal Raft core.
///
/// For example:
/// ```test
///                                 .-> VoteRequest   ---.
/// Leader --> RaftNetwork -- TCP --+-> AppendEntries ---+-- --> Server --> ProtocolApi -> RaftCore
///                                 '-> InstallSnapshot -'
/// ----------------------                                       ----------------------------------
/// Remote Leader Node                                           Local Follower Node
/// ```
#[since(version = "0.10.0")]
pub(crate) struct ProtocolApi<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    inner: Arc<RaftInner<C>>,

    /// Sender to the state machine worker, for requests that RaftCore just forwards.
    sm_cmd_tx: MpscWeakSenderOf<C, sm::Command<C, SM>>,

    /// Sender of the dedicated channel that delivers a full snapshot to RaftCore.
    ///
    /// The snapshot data type is defined by the state machine, thus it does not go through
    /// the [`RaftMsg`] channel, which is independent of the state machine type.
    install_snapshot_tx: MpscSenderOf<C, InstallFullSnapshotRequest<C, SM>>,
}

impl<C, SM> ProtocolApi<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(in crate::raft) fn new(
        inner: Arc<RaftInner<C>>,
        sm_cmd_tx: MpscWeakSenderOf<C, sm::Command<C, SM>>,
        install_snapshot_tx: MpscSenderOf<C, InstallFullSnapshotRequest<C, SM>>,
    ) -> Self {
        Self {
            inner,
            sm_cmd_tx,
            install_snapshot_tx,
        }
    }

    /// Send a command to the state machine worker directly, without going through RaftCore.
    async fn send_sm_command(&self, cmd: sm::Command<C, SM>) -> Result<(), Fatal<C>> {
        let Some(tx) = self.sm_cmd_tx.upgrade() else {
            return Err(self.inner.get_core_stop_error().await);
        };

        let send_res = tx.send(cmd).await;
        if send_res.is_err() {
            return Err(self.inner.get_core_stop_error().await);
        }
        Ok(())
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub(crate) async fn vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, Fatal<C>> {
        tracing::info!("Raft::vote(): rpc: {}", rpc);

        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub(crate) async fn pre_vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, Fatal<C>> {
        tracing::info!("Raft::pre_vote(): rpc: {}", rpc);

        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::RequestPreVote { rpc, tx }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub(crate) async fn append_entries(
        &self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C>, Fatal<C>> {
        tracing::debug!("Raft::append_entries: rpc: {}", rpc);

        let (tx, rx) = C::oneshot();
        let stream_result: StreamAppendResult<C> = self.inner.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await?;
        Ok(AppendEntriesResponse::from(stream_result))
    }

    #[since(version = "0.10.0")]
    pub(crate) fn stream_append<S>(
        self,
        stream: S,
    ) -> impl Stream<Item = Result<StreamAppendResult<C>, Fatal<C>>> + OptionalSend + 'static
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + 'static,
    {
        stream_append::stream_append(self.inner, stream)
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn get_snapshot(&self) -> Result<Option<SnapshotOf<C, SnapshotDataOf<C, SM>>>, Fatal<C>> {
        tracing::debug!("Raft::get_snapshot()");

        let (tx, rx) = C::oneshot();
        self.send_sm_command(sm::Command::get_snapshot(tx)).await?;
        self.inner.recv_msg(rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn begin_receiving_snapshot(&self) -> Result<SnapshotDataOf<C, SM>, Fatal<C>> {
        tracing::info!("Raft::begin_receiving_snapshot()");

        let (tx, rx) = C::oneshot();
        self.send_sm_command(sm::Command::begin_receiving_snapshot(tx)).await?;
        self.inner.recv_msg(rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn install_full_snapshot(
        &self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, SnapshotDataOf<C, SM>>,
    ) -> Result<SnapshotResponse<C>, Fatal<C>> {
        tracing::info!("Raft::install_full_snapshot()");

        let (tx, rx) = C::oneshot();
        let req = InstallFullSnapshotRequest { vote, snapshot, tx };

        let send_res = self.install_snapshot_tx.send(req).await;
        if send_res.is_err() {
            return Err(self.inner.get_core_stop_error().await);
        }

        self.inner.recv_msg(rx).await
    }

    #[since(version = "0.10.0")]
    pub(crate) async fn handle_transfer_leader(
        &self,
        req: TransferLeaderRequest<C>,
    ) -> Result<TransferLeaderResponse<C>, Fatal<C>> {
        // Reset the Leader lease at once and quit if this is not the assigned next leader.
        // Only the assigned next Leader waits for the log to be flushed.
        if &req.to_node_id == self.inner.id()
            && let Err(err) = self.ensure_log_flushed_for_transfer_leader(&req).await?
        {
            return Ok(Err(err));
        }

        let raft_msg = RaftMsg::HandleTransferLeader {
            from: req.from_leader.clone(),
            to: req.to_node_id.clone(),
            last_log_id: req.last_log_id.clone(),
        };

        self.inner.send_msg(raft_msg).await?;

        Ok(Ok(()))
    }

    fn check_transfer_leader_progress(
        req: &TransferLeaderRequest<C>,
        progress: &Option<FlushPoint<C>>,
    ) -> TransferLeaderResponse<C> {
        let progress = if let Some(progress) = progress {
            progress
        } else {
            return Err(TransferLeaderError::LogNotFlushed {
                expected: req.last_log_id.clone(),
                actual: None,
            });
        };

        let actual_vote = progress.vote.as_ref_vote();

        let vote_matches = actual_vote == req.from_leader.as_ref_vote();
        let log_is_flushed = progress.last_log_id.as_ref() >= req.last_log_id.as_ref();

        if vote_matches && log_is_flushed {
            return Ok(());
        }

        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let vote_changed = !(req.from_leader.as_ref_vote() >= actual_vote);
        if vote_changed {
            let actual_vote =
                VoteOf::<C>::from_leader_id(progress.vote.leader_id().clone(), progress.vote.is_committed());

            return Err(TransferLeaderError::VoteChanged {
                expected: req.from_leader.clone(),
                actual: actual_vote,
            });
        }

        Err(TransferLeaderError::LogNotFlushed {
            expected: req.last_log_id.clone(),
            actual: progress.last_log_id.clone(),
        })
    }

    fn log_transfer_leader_progress(
        req: &TransferLeaderRequest<C>,
        progress: &Option<FlushPoint<C>>,
        res: &TransferLeaderResponse<C>,
    ) {
        match res {
            Ok(()) => {
                tracing::info!(
                    "Leader-transfer condition satisfied, submit Leader-transfer message; \
                     expected: (vote: {}, flushed_log: {})",
                    req.from_leader,
                    req.last_log_id.display(),
                );
            }
            Err(TransferLeaderError::VoteChanged { .. }) => {
                tracing::warn!(
                    "Vote changed, give up Leader-transfer; expected vote: {}, progress: {}",
                    req.from_leader,
                    progress.display()
                );
            }
            Err(_) => {
                tracing::warn!(
                    "Leader-transfer condition failed, reject Leader-transfer; \
                     expected: (vote: {}; flushed_log: {}), progress: {}",
                    req.from_leader,
                    req.last_log_id.display(),
                    progress.display()
                );
            }
        }
    }

    /// Wait for the log to be flushed to make sure the RequestVote.last_log_id is up to date, then
    /// TransferLeader will be able to proceed.
    async fn ensure_log_flushed_for_transfer_leader(
        &self,
        req: &TransferLeaderRequest<C>,
    ) -> Result<TransferLeaderResponse<C>, Fatal<C>> {
        // If the next Leader is this node, wait for the log to be flushed to make sure the
        // RequestVote.last_log_id is up to date.

        let timeout_at = C::now() + Duration::from_millis(self.inner.config().election_timeout_min);
        let mut log_progress = self.inner.progress_watcher.log_progress();

        loop {
            let progress = log_progress.get();
            let res = Self::check_transfer_leader_progress(req, &progress);

            if let Ok(()) | Err(TransferLeaderError::VoteChanged { .. }) = &res {
                Self::log_transfer_leader_progress(req, &progress, &res);
                return Ok(res);
            }

            let changed_res = C::timeout_at(timeout_at, log_progress.changed()).await;
            let changed_res = match changed_res {
                Ok(changed_res) => changed_res,
                Err(_) => {
                    let progress = log_progress.get();
                    let res = Self::check_transfer_leader_progress(req, &progress);
                    Self::log_transfer_leader_progress(req, &progress, &res);
                    return Ok(res);
                }
            };

            if changed_res.is_err() {
                return Err(self.inner.get_core_stop_error().await);
            }
        }
    }
}
