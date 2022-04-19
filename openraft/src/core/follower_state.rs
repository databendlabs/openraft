use tokio::time::sleep_until;
use tracing::Instrument;

use crate::core::MetricsProvider;
use crate::core::RaftCore;
use crate::core::ServerState;
use crate::error::ExtractFatal;
use crate::error::Fatal;
use crate::raft::RaftMsg;
use crate::summary::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::Update;

/// Volatile state specific to a Raft node in follower state.
pub(crate) struct FollowerState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    core: &'a mut RaftCore<C, N, S>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> MetricsProvider<C::NodeId>
    for FollowerState<'a, C, N, S>
{
    // the non-leader state use the default impl of `get_leader_metrics_option`
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> FollowerState<'a, C, N, S> {
    pub(crate) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        Self { core }
    }

    /// Run the follower loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="follower"))]
    pub(crate) async fn run(self) -> Result<(), Fatal<C::NodeId>> {
        self.follower_loop().await?;
        Ok(())
    }

    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="follower"))]
    async fn follower_loop(mut self) -> Result<(), Fatal<C::NodeId>> {
        // report the new state before enter the loop
        self.core.report_metrics(Update::Update(None));

        loop {
            if !self.core.engine.state.server_state.is_follower() {
                return Ok(());
            }

            self.core.report_metrics_if_needed(&self);
            self.core.engine.metrics_flags.reset();

            let election_timeout = sleep_until(self.core.get_next_election_timeout()); // Value is updated as heartbeats are received.

            tokio::select! {
                // If an election timeout is hit, then we need to transition to candidate.
                _ = election_timeout => {
                    tracing::debug!("timeout to recv a event, change to CandidateState");
                    self.core.set_target_state(ServerState::Candidate)
                },

                Some((msg,span)) = self.core.rx_api.recv() => {
                    self.handle_msg(msg).instrument(span).await?;
                },

                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),

                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(ServerState::Shutdown),
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "follower", id=display(self.core.id)))]
    pub(crate) async fn handle_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
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
                req(ServerState::Follower, &mut self.core.storage, &mut self.core.network);
            }
        };
        Ok(())
    }
}
