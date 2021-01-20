use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing_futures::Instrument;

use crate::core::{CandidateState, RaftCore, State, UpdateCurrentLeader};
use crate::error::RaftResult;
use crate::raft::{VoteRequest, VoteResponse};
use crate::{AppData, AppDataResponse, NodeId, RaftNetwork, RaftStorage};

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    /// An RPC invoked by candidates to gather votes (§5.2).
    ///
    /// See `receiver implementation: RequestVote RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level = "trace", skip(self, msg))]
    pub(super) async fn handle_vote_request(&mut self, msg: VoteRequest) -> RaftResult<VoteResponse> {
        // If candidate's current term is less than this nodes current term, reject.
        if msg.term < self.current_term {
            tracing::trace!({candidate=msg.candidate_id, self.current_term, rpc_term=msg.term}, "RequestVote RPC term is less than current term");
            return Ok(VoteResponse {
                term: self.current_term,
                vote_granted: false,
            });
        }

        // Do not respond to the request if we've received a heartbeat within the election timeout minimum.
        if let Some(inst) = &self.last_heartbeat {
            let now = Instant::now();
            let delta = now.duration_since(*inst);
            if self.config.election_timeout_min >= (delta.as_millis() as u64) {
                tracing::trace!(
                    { candidate = msg.candidate_id },
                    "rejecting vote request received within election timeout minimum"
                );
                return Ok(VoteResponse {
                    term: self.current_term,
                    vote_granted: false,
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
        let client_is_uptodate = (msg.last_log_term >= self.last_log_term) && (msg.last_log_index >= self.last_log_index);
        if !client_is_uptodate {
            tracing::trace!(
                { candidate = msg.candidate_id },
                "rejecting vote request as candidate's log is not up-to-date"
            );
            return Ok(VoteResponse {
                term: self.current_term,
                vote_granted: false,
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
            }),
            // This node has already voted for a different candidate.
            Some(_) => Ok(VoteResponse {
                term: self.current_term,
                vote_granted: false,
            }),
            // This node has not yet voted for the current term, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                self.set_target_state(State::Follower);
                self.update_next_election_timeout(false);
                self.save_hard_state().await?;
                tracing::trace!({candidate=msg.candidate_id, msg.term}, "voted for candidate");
                Ok(VoteResponse {
                    term: self.current_term,
                    vote_granted: true,
                })
            }
        }
    }
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> CandidateState<'a, D, R, N, S> {
    /// Handle response from a vote request sent to a peer.
    #[tracing::instrument(level = "trace", skip(self, res, target))]
    pub(super) async fn handle_vote_response(&mut self, res: VoteResponse, target: NodeId) -> RaftResult<()> {
        // If peer's term is greater than current term, revert to follower state.
        if res.term > self.core.current_term {
            self.core.update_current_term(res.term, None);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            self.core.set_target_state(State::Follower);
            self.core.save_hard_state().await?;
            tracing::trace!("reverting to follower state due to greater term observed in RequestVote RPC response");
            return Ok(());
        }

        // If peer granted vote, then update campaign state.
        if res.vote_granted {
            // Handle vote responses from the C0 config group.
            if self.core.membership.members.contains(&target) {
                self.votes_granted_old += 1;
            }
            // Handle vote responses from members of C1 config group.
            if self
                .core
                .membership
                .members_after_consensus
                .as_ref()
                .map(|members| members.contains(&target))
                .unwrap_or(false)
            {
                self.votes_granted_new += 1;
            }
            // If we've received enough votes from both config groups, then transition to leader state`.
            if self.votes_granted_old >= self.votes_needed_old && self.votes_granted_new >= self.votes_needed_new {
                tracing::trace!("transitioning to leader state as minimum number of votes have been received");
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
        let all_members = self.core.membership.all_nodes();
        let (tx, rx) = mpsc::channel(all_members.len());
        for member in all_members.into_iter().filter(|member| member != &self.core.id) {
            let rpc = VoteRequest::new(self.core.current_term, self.core.id, self.core.last_log_index, self.core.last_log_term);
            let (network, tx_inner) = (self.core.network.clone(), tx.clone());
            let _ = tokio::spawn(
                async move {
                    match network.vote(member, rpc).await {
                        Ok(res) => {
                            let _ = tx_inner.send((res, member)).await;
                        }
                        Err(err) => tracing::error!({error=%err, peer=member}, "error while requesting vote from peer"),
                    }
                }
                .instrument(tracing::trace_span!("requesting vote from peer", target = member)),
            );
        }
        rx
    }
}
