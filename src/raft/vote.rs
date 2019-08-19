use actix::prelude::*;

use crate::{
    AppError, NodeId,
    common::{DependencyAddr, UpdateCurrentLeader},
    messages::{VoteRequest, VoteResponse},
    network::RaftNetwork,
    raft::{RaftState, Raft},
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
            return Box::new(fut::err(()));
        }

        Box::new(fut::result(self.handle_vote_request(ctx, msg)))
    }
}

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Raft<E, N, S> {
    /// Business logic of handling a `VoteRequest` RPC.
    fn handle_vote_request(&mut self, ctx: &mut Context<Self>, msg: VoteRequest) -> Result<VoteResponse, ()> {
        // Don't interact with non-cluster members.
        if !self.membership.contains(&msg.candidate_id) {
            return Ok(VoteResponse{term: self.current_term, vote_granted: false, is_candidate_unknown: true});
        }

        // If candidate's current term is less than this nodes current term, reject.
        if &msg.term < &self.current_term {
            return Ok(VoteResponse{term: self.current_term, vote_granted: false, is_candidate_unknown: false});
        }

        // Per spec, if we observe a term greater than our own, we must update
        // term & immediately become follower, we still need to do vote checking after this.
        if &msg.term > &self.current_term {
            self.update_current_term(msg.term, None);
            self.save_hard_state(ctx);
        }

        // Check if candidate's log is at least as up-to-date as this node's.
        // If candidate's log is not at least as up-to-date as this node, then reject.
        let client_is_uptodate = (&msg.last_log_term >= &self.last_log_term) && (&msg.last_log_index >= &self.last_log_index);
        if !client_is_uptodate {
            return Ok(VoteResponse{term: self.current_term, vote_granted: false, is_candidate_unknown: false});
        }

        // Candidate's log is up-to-date so handle voting conditions.
        match &self.voted_for {
            // This node has already voted for the candidate.
            Some(candidate_id) if candidate_id == &msg.candidate_id => {
                Ok(VoteResponse{term: self.current_term, vote_granted: true, is_candidate_unknown: false})
            }
            // This node has already voted for a different candidate.
            Some(_) => Ok(VoteResponse{term: self.current_term, vote_granted: false, is_candidate_unknown: false}),
            // This node has not already voted, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                self.save_hard_state(ctx);
                self.update_election_timeout(ctx);
                self.become_follower(ctx);
                Ok(VoteResponse{term: self.current_term, vote_granted: true, is_candidate_unknown: false})
            },
        }
    }

    /// Request a vote from the the target peer.
    pub(super) fn request_vote(&mut self, _: &mut Context<Self>, target: NodeId) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        let rpc = VoteRequest::new(target, self.current_term, self.id, self.last_log_index, self.last_log_term);
        fut::wrap_future(self.network.send(rpc))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
            .and_then(|res, _, _| fut::result(res))
            .and_then(move |res, act, ctx| {
                // Ensure the node is still in candidate state.
                let state = match &mut act.state {
                    RaftState::Candidate(state) => state,
                    _ => {
                        return fut::ok(());
                    }
                };

                // If responding node sees this node as being unknown to the cluster, and this
                // node has been active, then go into NonVoter state, as this typically means this
                // node is being removed from the cluster.
                if res.is_candidate_unknown && act.last_log_index > 0 {
                    act.become_non_voter(ctx);
                    return fut::ok(());
                }

                // If peer's term is greater than current term, revert to follower state.
                if res.term > act.current_term {
                    act.update_current_term(res.term, None);
                    act.update_current_leader(ctx, UpdateCurrentLeader::Unknown);
                    act.become_follower(ctx);
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
