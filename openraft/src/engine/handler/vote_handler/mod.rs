use crate::engine::engine_impl::EngineOutput;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::time_state::TimeState;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::error::RejectVoteRequest;
use crate::internal_server_state::InternalServerState;
use crate::leader::Leader;
use crate::progress::Progress;
use crate::raft_state::LogStateReader;
use crate::LogIdOptionExt;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::Vote;

#[cfg(test)] mod handle_message_vote_test;

/// Handle raft vote related operations
///
/// A `vote` defines the state of a openraft node.
/// See [`RaftState::calc_server_state`] .
pub(crate) struct VoteHandler<'st, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: &'st EngineConfig<NID>,
    pub(crate) state: &'st mut RaftState<NID, N>,
    pub(crate) timer: &'st mut TimeState,
    pub(crate) output: &'st mut EngineOutput<NID, N>,
    pub(crate) internal_server_state: &'st mut InternalServerState<NID>,
}

impl<'st, NID, N> VoteHandler<'st, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Mark the vote as committed, i.e., being granted and saved by a quorum.
    ///
    /// The committed vote, is not necessary in original raft.
    /// Openraft insists doing this because:
    /// - Voting is not in the hot path, thus no performance penalty.
    /// - Leadership won't be lost if a leader restarted quick enough.
    pub(crate) fn commit_vote(&mut self) {
        debug_assert!(!self.state.vote_ref().is_committed());
        debug_assert_eq!(
            self.state.vote_ref().leader_id().voted_for(),
            Some(self.config.id),
            "it can only commit its own vote"
        );

        let mut v = *self.state.vote_ref();
        v.commit();

        let _res = self.handle_message_vote(&v);
        debug_assert!(_res.is_ok(), "commit vote can not fail but: {:?}", _res);
    }

    /// Check and update the local vote and related state for every message received.
    ///
    /// This is used by all incoming event, such as the 3 RPC append-entries, vote, install-snapshot
    /// to check the `vote` field.
    ///
    /// Grant vote if vote >= mine.
    /// Note: This method does not check last-log-id. handle-vote-request has to deal with
    /// last-log-id itself.
    pub(crate) fn handle_message_vote(&mut self, vote: &Vote<NID>) -> Result<(), RejectVoteRequest<NID>> {
        // Partial ord compare:
        // Vote does not has to be total ord.
        // `!(a >= b)` does not imply `a < b`.
        if vote >= self.state.vote_ref() {
            // Ok
        } else {
            return Err(RejectVoteRequest::ByVote(*self.state.vote_ref()));
        }
        tracing::debug!(%vote, "vote is changing to" );

        // Grant the vote

        if vote > self.state.vote_ref() {
            self.state.vote.update(*self.timer.now(), *vote);
            self.output.push_command(Command::SaveVote { vote: *vote });
        } else {
            self.state.vote.touch(*self.timer.now());
        }

        // Update vote related timer and lease.

        tracing::debug!(now = debug(&self.timer.now()), "{}", func_name!());

        self.update_internal_server_state();

        Ok(())
    }

    /// Enter leading or following state by checking `vote`.
    pub(crate) fn update_internal_server_state(&mut self) {
        if self.state.vote_ref().leader_id().voted_for() == Some(self.config.id) {
            self.become_leading();
        } else {
            self.become_following();
        }
    }

    /// Enter leading state(vote.node_id == self.id) .
    ///
    /// Create a new leading state, when raft enters candidate state.
    /// Leading state has two phase: election phase and replication phase, similar to paxos phase-1
    /// and phase-2. Leader and Candidate shares the same state.
    pub(crate) fn become_leading(&mut self) {
        if let Some(l) = self.internal_server_state.leading_mut() {
            if l.vote.leader_id() == self.state.vote_ref().leader_id() {
                // Vote still belongs to the same leader. Just updating vote is enough.
                l.vote = *self.state.vote_ref();
                self.server_state_handler().update_server_state_if_changed();
                return;
            }
        }

        // It's a different leader that creates this vote.
        // Re-create a new Leader instance.

        let em = &self.state.membership_state.effective();
        let mut leader = Leader::new(
            *self.state.vote_ref(),
            em.membership().to_quorum_set(),
            em.learner_ids(),
            self.state.last_log_id().index(),
        );

        // We can just ignore the result here:
        // The `committed` will not be updated until a log of current term is granted by a quorum
        let _ = leader.progress.update_with(&self.config.id, |v| v.matching = self.state.last_log_id().copied());

        *self.internal_server_state = InternalServerState::Leading(leader);

        self.server_state_handler().update_server_state_if_changed();
    }

    /// Enter following state(vote.node_id != self.id or self is not a voter).
    ///
    /// This node then becomes raft-follower or raft-learner.
    pub(crate) fn become_following(&mut self) {
        // TODO: entering following needs to check last-log-id on other node to decide the election
        // timeout.

        debug_assert!(
            self.state.vote_ref().leader_id().voted_for() != Some(self.config.id)
                || !self.state.membership_state.effective().membership().is_voter(&self.config.id),
            "It must hold: vote is not mine, or I am not a voter(leader just left the cluster)"
        );

        if self.internal_server_state.is_following() {
            return;
        }

        *self.internal_server_state = InternalServerState::Following;

        self.server_state_handler().update_server_state_if_changed();
    }

    pub(crate) fn server_state_handler(&mut self) -> ServerStateHandler<NID, N> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}
