use std::collections::BTreeSet;

use maplit::btreeset;
use tokio::time::sleep_until;
use tracing::Instrument;

use crate::core::MetricsProvider;
use crate::core::RaftCore;
use crate::core::ServerState;
use crate::error::ExtractFatal;
use crate::error::Fatal;
use crate::raft::RaftMsg;
use crate::raft::VoteResponse;
use crate::summary::MessageSummary;
use crate::vote::Vote;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::Update;

/// Volatile state specific to a Raft node in candidate state.
pub(crate) struct CandidateState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    pub(crate) core: &'a mut RaftCore<C, N, S>,

    /// Ids of the nodes that has granted our vote request.
    pub(crate) granted: BTreeSet<C::NodeId>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> MetricsProvider<C::NodeId>
    for CandidateState<'a, C, N, S>
{
    // the non-leader state use the default impl of `get_leader_metrics_option`
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> CandidateState<'a, C, N, S> {
    pub(crate) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        Self {
            core,
            granted: btreeset! {},
        }
    }

    /// Run the candidate loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="candidate"))]
    pub(crate) async fn run(self) -> Result<(), Fatal<C::NodeId>> {
        // Each iteration of the outer loop represents a new term.
        self.candidate_loop().await?;
        Ok(())
    }

    async fn candidate_loop(mut self) -> Result<(), Fatal<C::NodeId>> {
        // report the new state before enter the loop
        self.core.report_metrics(Update::Update(None));

        loop {
            if !self.core.engine.state.server_state.is_candidate() {
                return Ok(());
            }

            self.core.report_metrics_if_needed(&self);
            self.core.engine.metrics_flags.reset();

            // Setup new term.
            self.core.update_next_election_timeout(false); // Generates a new rand value within range.

            self.core.engine.state.vote = Vote::new(self.core.engine.state.vote.term + 1, self.core.id);

            self.core.save_vote().await?;

            // vote for itself.
            self.handle_vote_response(
                VoteResponse {
                    vote: self.core.engine.state.vote,
                    vote_granted: true,
                    last_log_id: self.core.engine.state.last_log_id,
                },
                self.core.id,
            )
            .await?;
            if !self.core.engine.state.server_state.is_candidate() {
                return Ok(());
            }

            // Send RPCs to all members in parallel.
            let mut pending_votes = self.spawn_parallel_vote_requests().await;

            // Inner processing loop for this Raft state.
            loop {
                if !self.core.engine.state.server_state.is_candidate() {
                    return Ok(());
                }

                let timeout_fut = sleep_until(self.core.get_next_election_timeout());

                let span = tracing::debug_span!("CHrx:CandidateState");
                let _ent = span.enter();

                tokio::select! {
                    _ = timeout_fut => break, // This election has timed-out. Break to outer loop, which starts a new term.

                    Some((res, peer)) = pending_votes.recv() => {
                        self.handle_vote_response(res, peer).await?;
                    },

                    Some((msg,span)) = self.core.rx_api.recv() => {
                        self.handle_msg(msg).instrument(span).await?;
                    },

                    Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),

                    Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(ServerState::Shutdown),
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "candidate", id=display(self.core.id)))]
    pub async fn handle_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());
        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let _ = tx.send(self.core.handle_append_entries_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let _ = tx.send(self.core.handle_vote_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::CheckIsLeaderRequest { tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ClientWriteRequest { rpc: _, tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::Initialize { tx, .. } => {
                self.core.reject_init_with_config(tx);
            }
            RaftMsg::AddLearner { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ChangeMembership { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ExternalRequest { req } => {
                req(ServerState::Candidate, &mut self.core.storage, &mut self.core.network);
            }
        };
        Ok(())
    }
}
