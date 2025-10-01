use std::time::Duration;

use openraft_macros::since;

use crate::LogIdOptionExt;
use crate::LogIndexOptionExt;
use crate::RaftMetrics;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::display_ext::DisplayOptionExt;
use crate::error::Fatal;
#[cfg(doc)]
use crate::error::into_raft_result::IntoRaftResult;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::TransferLeaderRequest;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::raft_inner::RaftInner;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::SnapshotDataOf;
use crate::type_config::alias::VoteOf;
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
pub(crate) struct ProtocolApi<'a, C>
where C: RaftTypeConfig
{
    inner: &'a RaftInner<C>,
}

impl<'a, C> ProtocolApi<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn new(inner: &'a RaftInner<C>) -> Self {
        Self { inner }
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub(crate) async fn vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, Fatal<C>> {
        tracing::info!(rpc = display(&rpc), "Raft::vote()");

        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub(crate) async fn append_entries(
        &self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C>, Fatal<C>> {
        tracing::debug!(rpc = display(&rpc), "Raft::append_entries");

        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn get_snapshot(&self) -> Result<Option<Snapshot<C>>, Fatal<C>> {
        tracing::debug!("Raft::get_snapshot()");

        let (tx, rx) = C::oneshot();
        let cmd = ExternalCommand::GetSnapshot { tx };
        self.inner.call_core(RaftMsg::ExternalCommand { cmd }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn begin_receiving_snapshot(&self) -> Result<SnapshotDataOf<C>, Fatal<C>> {
        tracing::info!("Raft::begin_receiving_snapshot()");

        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::BeginReceivingSnapshot { tx }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn install_full_snapshot(
        &self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
    ) -> Result<SnapshotResponse<C>, Fatal<C>> {
        tracing::info!("Raft::install_full_snapshot()");

        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::InstallFullSnapshot { vote, snapshot, tx }, rx).await
    }

    #[since(version = "0.10.0")]
    pub(crate) async fn handle_transfer_leader(&self, req: TransferLeaderRequest<C>) -> Result<(), Fatal<C>> {
        // Reset the Leader lease at once and quit if this is not the assigned next leader.
        // Only the assigned next Leader waits for the log to be flushed.
        if &req.to_node_id == self.inner.id() {
            self.ensure_log_flushed_for_transfer_leader(&req).await?;
        }

        let raft_msg = RaftMsg::HandleTransferLeader {
            from: req.from_leader,
            to: req.to_node_id,
        };

        self.inner.send_msg(raft_msg).await?;

        Ok(())
    }

    /// Wait for the log to be flushed to make sure the RequestVote.last_log_id is up to date, then
    /// TransferLeader will be able to proceed.
    async fn ensure_log_flushed_for_transfer_leader(&self, req: &TransferLeaderRequest<C>) -> Result<(), Fatal<C>> {
        // If the next Leader is this node, wait for the log to be flushed to make sure the
        // RequestVote.last_log_id is up to date.

        // Condition satisfied to become Leader
        let ok = |m: &RaftMetrics<C>| {
            req.from_leader() == &m.vote && m.last_log_index.next_index() >= req.last_log_id().next_index()
        };

        // Condition failed to become Leader
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let fail = |m: &RaftMetrics<C>| !(req.from_leader.as_ref_vote() >= m.vote.as_ref_vote());

        let timeout = Some(Duration::from_millis(self.inner.config().election_timeout_min));
        let metrics_res =
            self.inner.wait(timeout).metrics(|st| ok(st) || fail(st), "transfer_leader await flushed log").await;

        match metrics_res {
            Ok(metrics) => {
                if fail(&metrics) {
                    tracing::warn!(
                        "Vote changed, give up Leader-transfer; expected vote: {}, metrics: {}",
                        req.from_leader,
                        metrics
                    );
                    return Ok(());
                }
                tracing::info!(
                    "Leader-transfer condition satisfied, submit Leader-transfer message; \
                     expected: (vote: {}, flushed_log: {})",
                    req.from_leader,
                    req.last_log_id.display(),
                );
            }
            Err(err) => {
                tracing::warn!(
                    "Leader-transfer condition fail to satisfy, still submit Leader-transfer; \
                    expected: (vote: {}; flushed_log: {}), error: {}",
                    req.from_leader,
                    req.last_log_id.display(),
                    err
                );
            }
        };

        Ok(())
    }
}
