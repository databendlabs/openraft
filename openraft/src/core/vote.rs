use tokio::time::Instant;

use crate::core::RaftCore;
use crate::core::ServerState;
use crate::error::VoteError;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::summary::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// An RPC invoked by candidates to gather votes (§5.2).
    ///
    /// See `receiver implementation: RequestVote RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level = "debug", skip(self, req), fields(req=%req.summary()))]
    pub(super) async fn handle_vote_request(
        &mut self,
        req: VoteRequest<C::NodeId>,
    ) -> Result<VoteResponse<C::NodeId>, VoteError<C::NodeId>> {
        tracing::debug!(
            %req.vote,
            ?self.engine.state.vote,
            "start handle_vote_request"
        );
        let last_log_id = self.engine.state.last_log_id;

        if req.vote < self.engine.state.vote {
            tracing::debug!(
                %req.vote,
                ?self.engine.state.vote,
                "RequestVote RPC term is less than current term"
            );
            return Ok(VoteResponse {
                vote: self.engine.state.vote,
                vote_granted: false,
                last_log_id,
            });
        }

        // Do not respond to the request if we've received a heartbeat within the election timeout minimum.
        if let Some(inst) = &self.last_heartbeat {
            let now = Instant::now();
            let delta = now.duration_since(*inst);
            if self.config.election_timeout_min >= (delta.as_millis() as u64) {
                tracing::debug!(
                    %req.vote,
                    ?delta,
                    "rejecting vote request received within election timeout minimum"
                );
                return Ok(VoteResponse {
                    vote: self.engine.state.vote,
                    vote_granted: false,
                    last_log_id,
                });
            }
        }

        // Check if candidate's log is at least as up-to-date as this node's.
        // If candidate's log is not at least as up-to-date as this node, then reject.
        if req.last_log_id < last_log_id {
            tracing::debug!(
                %req.vote,
                "rejecting vote request as candidate's log is not up-to-date"
            );
            return Ok(VoteResponse {
                vote: self.engine.state.vote,
                vote_granted: false,
                last_log_id,
            });
        }

        self.update_next_election_timeout(false);
        self.engine.state.vote = req.vote;
        self.save_vote().await?;

        self.set_target_state(ServerState::Follower);

        tracing::debug!(%req.vote, "voted for candidate");

        Ok(VoteResponse {
            vote: self.engine.state.vote,
            vote_granted: true,
            last_log_id,
        })
    }
}
