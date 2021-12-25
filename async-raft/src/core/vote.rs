use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing_futures::Instrument;

use crate::core::CandidateState;
use crate::core::RaftCore;
use crate::core::State;
use crate::core::UpdateCurrentLeader;
use crate::error::RaftResult;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::AppData;
use crate::AppDataResponse;
use crate::NodeId;
use crate::RaftNetwork;
use crate::RaftStorage;

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    /// An RPC invoked by candidates to gather votes (§5.2).
    ///
    /// See `receiver implementation: RequestVote RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn handle_vote_request(&mut self, msg: VoteRequest) -> RaftResult<VoteResponse> {
        tracing::debug!({candidate=msg.candidate_id, self.current_term, rpc_term=msg.term}, "start handle_vote_request");

        // If candidate's current term is less than this nodes current term, reject.
        if msg.term < self.current_term {
            tracing::debug!({candidate=msg.candidate_id, self.current_term, rpc_term=msg.term}, "RequestVote RPC term is less than current term");
            return Ok(VoteResponse {
                term: self.current_term,
                vote_granted: false,
                last_log_id: self.last_log_id,
            });
        }

        // Do not respond to the request if we've received a heartbeat within the election timeout minimum.
        if let Some(inst) = &self.last_heartbeat {
            let now = Instant::now();
            let delta = now.duration_since(*inst);
            if self.config.election_timeout_min >= (delta.as_millis() as u64) {
                tracing::debug!(
                    { candidate = msg.candidate_id },
                    "rejecting vote request received within election timeout minimum"
                );
                return Ok(VoteResponse {
                    term: self.current_term,
                    vote_granted: false,
                    last_log_id: self.last_log_id,
                });
            }
        }

        // Per spec, if we observe a term greater than our own outside of the election timeout
        // minimum, then we must update term & immediately become follower. We still need to
        // do vote checking after this.
        if msg.term > self.current_term {
            self.update_current_term(msg.term, None);
            self.update_next_election_timeout(false);
            self.set_target_state(State::Follower);
            self.save_hard_state().await?;
        }

        // Check if candidate's log is at least as up-to-date as this node's.
        // If candidate's log is not at least as up-to-date as this node, then reject.
        if msg.last_log_id < self.last_log_id {
            tracing::debug!(
                { candidate = msg.candidate_id },
                "rejecting vote request as candidate's log is not up-to-date"
            );
            return Ok(VoteResponse {
                term: self.current_term,
                vote_granted: false,
                last_log_id: self.last_log_id,
            });
        }

        // TODO: add hook for PreVote optimization here. If the RPC is a PreVote, then at this
        // point we can respond to the candidate telling them that we would vote for them.

        // Candidate's log is up-to-date so handle voting conditions.
        match &self.voted_for {
            // This node has already voted for the candidate.
            Some(candidate_id) if candidate_id == &msg.candidate_id => Ok(VoteResponse {
                term: self.current_term,
                vote_granted: true,
                last_log_id: self.last_log_id,
            }),
            // This node has already voted for a different candidate.
            Some(_) => Ok(VoteResponse {
                term: self.current_term,
                vote_granted: false,
                last_log_id: self.last_log_id,
            }),
            // This node has not yet voted for the current term, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                self.set_target_state(State::Follower);
                self.update_next_election_timeout(false);
                self.save_hard_state().await?;
                tracing::debug!({candidate=msg.candidate_id, msg.term}, "voted for candidate");
                Ok(VoteResponse {
                    term: self.current_term,
                    vote_granted: true,
                    last_log_id: self.last_log_id,
                })
            }
        }
    }
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> CandidateState<'a, D, R, N, S> {
    /// Handle response from a vote request sent to a peer.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn handle_vote_response(&mut self, res: VoteResponse, target: NodeId) -> RaftResult<()> {
        // If peer's term is greater than current term, revert to follower state.

        if res.term > self.core.current_term {
            self.core.update_current_term(res.term, None);
            self.core.save_hard_state().await?;

            self.core.update_current_leader(UpdateCurrentLeader::Unknown);

            // If a quorum of nodes have higher `last_log_id`, I have no chance to become a leader.
            // TODO(xp): This is a simplified impl: revert to follower as soon as seeing a higher `last_log_id`.
            //           When reverted to follower, it waits for heartbeat for 2 second before starting a new round of
            //           election.
            if self.core.last_log_id < res.last_log_id {
                self.core.set_target_state(State::Follower);
                tracing::debug!("reverting to follower state due to greater term observed in RequestVote RPC response");
            } else {
                tracing::debug!(
                    id = %self.core.id,
                    self_term=%self.core.current_term,
                    res_term=%res.term,
                    self_last_log_id=%self.core.last_log_id,
                    res_last_log_id=%res.last_log_id,
                    "I have lower term but higher or euqal last_log_id, keep trying to elect"
                );
            }
            return Ok(());
        }

        if res.vote_granted {
            self.granted.insert(target);

            if self.core.membership.membership.is_majority(&self.granted) {
                tracing::debug!("transitioning to leader state as minimum number of votes have been received");
                self.core.set_target_state(State::Leader);
                return Ok(());
            }
        }

        // Otherwise, we just return and let the candidate loop wait for more votes to come in.
        Ok(())
    }

    /// Spawn parallel vote requests to all cluster members.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn spawn_parallel_vote_requests(&self) -> mpsc::Receiver<(VoteResponse, NodeId)> {
        let all_members = self.core.membership.membership.all_nodes().clone();
        let (tx, rx) = mpsc::channel(all_members.len());
        for member in all_members.into_iter().filter(|member| member != &self.core.id) {
            let rpc = VoteRequest::new(
                self.core.current_term,
                self.core.id,
                self.core.last_log_id.index,
                self.core.last_log_id.term,
            );
            let (network, tx_inner) = (self.core.network.clone(), tx.clone());
            let _ = tokio::spawn(
                async move {
                    match network.send_vote(member, rpc).await {
                        Ok(res) => {
                            let _ = tx_inner.send((res, member)).await;
                        }
                        Err(err) => tracing::error!({error=%err, peer=member}, "error while requesting vote from peer"),
                    }
                }
                .instrument(tracing::debug_span!("requesting vote from peer", target = member)),
            );
        }
        rx
    }
}
