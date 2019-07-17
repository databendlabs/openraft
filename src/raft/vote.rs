use actix::prelude::*;
use log::{debug, warn};

use crate::{
    AppError, NodeId,
    messages::{VoteRequest, VoteResponse},
    network::RaftNetwork,
    raft::{RaftState, Raft, common::{DependencyAddr, UpdateCurrentLeader}},
    storage::RaftStorage,
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<VoteRequest> for Raft<E, N, S> {
    type Result = ResponseActFuture<Self, VoteResponse, ()>;

    /// An RPC invoked by candidates to gather votes (§5.2).
    ///
    /// Receiver implementation:
    ///
    /// 1. Reply `false` if `term` is less than receiver's current `term` (§5.1).
    /// 2. If receiver has not cast a vote for the current `term` or it voted for `candidate_id`, and
    ///    candidate’s log is atleast as up-to-date as receiver’s log, grant vote (§5.2, §5.4).
    fn handle(&mut self, msg: VoteRequest, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if let &RaftState::Initializing = &self.state {
            warn!("Received Raft RPC before initialization was complete.");
            return Box::new(fut::err(()));
        }

        Box::new(fut::result(self._handle_vote_request(ctx, msg)))
    }
}

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Raft<E, N, S> {
    /// Business logic of handling a `VoteRequest` RPC.
    fn _handle_vote_request(&mut self, ctx: &mut Context<Self>, msg: VoteRequest) -> Result<VoteResponse, ()> {
        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.candidate_id) {
            return Err(());
        }
        debug!("Handling vote request on node {} from node {} for term {}.", &self.id, &msg.candidate_id, &msg.term);

        // If candidate's current term is less than this nodes current term, reject.
        if &msg.term < &self.current_term {
            return Ok(VoteResponse{term: self.current_term, vote_granted: false});
        }

        // If candidate's log is not at least as up-to-date as this node, then reject.
        if &msg.last_log_term < &self.last_log_term || &msg.last_log_index < &self.last_log_index {
            return Ok(VoteResponse{term: self.current_term, vote_granted: false});
        }

        // Candidate's log is up-to-date so handle voting conditions. //

        // If term is newer than current term, cast vote.
        if &msg.term > &self.current_term {
            self.current_term = msg.term;
            self.voted_for = Some(msg.candidate_id);
            self.save_hard_state(ctx);
            self.update_election_timeout(ctx);
            return Ok(VoteResponse{term: self.current_term, vote_granted: true});
        }

        // Term is the same as current term. This will be rare, but could come about from some error conditions.
        match &self.voted_for {
            // This node has already voted for the candidate.
            Some(candidate_id) if candidate_id == &msg.candidate_id => {
                self.update_election_timeout(ctx);
                Ok(VoteResponse{term: self.current_term, vote_granted: true})
            }
            // This node has already voted for a different candidate.
            Some(_) => Ok(VoteResponse{term: self.current_term, vote_granted: false}),
            // This node has not already voted, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                self.save_hard_state(ctx);
                self.update_election_timeout(ctx);
                Ok(VoteResponse{term: self.current_term, vote_granted: true})
            },
        }
    }

    /// Request a vote from the the target peer.
    pub(super) fn request_vote(
        &mut self, _: &mut Context<Self>, target: NodeId, term: u64, last_log_index: u64, last_log_term: u64,
    ) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        let rpc = VoteRequest::new(target, term, self.id, last_log_index, last_log_term);
        fut::wrap_future(self.network.send(rpc))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
            .and_then(|res, _, _| fut::result(res))
            .and_then(|res, act, ctx| {
                // Ensure the node is still in candidate state.
                let state = match &mut act.state {
                    RaftState::Candidate(state) => state,
                    // If this node is not currently in candidate state, then this request is done.
                    _ => return fut::ok(()),
                };

                // If peer's term is greater than current term, revert to follower state.
                if res.term > act.current_term {
                    act.become_follower(ctx);
                    act.current_term = res.term;
                    act.update_current_leader(ctx, UpdateCurrentLeader::Unknown);
                    act.save_hard_state(ctx);
                    return fut::ok(());
                }

                // If peer granted vote, then update campaign state.
                if res.vote_granted {
                    state.votes_granted += 1;
                    if state.votes_granted >= state.votes_needed {
                        // If the campaign was successful, go into leader state.
                        act.become_leader(ctx);
                    }
                }

                fut::ok(())
            })
    }
}
