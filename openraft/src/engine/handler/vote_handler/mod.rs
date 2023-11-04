use std::fmt::Debug;

use crate::core::raft_msg::ResultSender;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::Respond;
use crate::engine::ValueSender;
use crate::error::RejectVoteRequest;
use crate::internal_server_state::InternalServerState;
use crate::leader::Leading;
use crate::raft_state::LogStateReader;
use crate::utime::UTime;
use crate::AsyncRuntime;
use crate::Instant;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::Vote;

#[cfg(test)] mod accept_vote_test;
#[cfg(test)] mod handle_message_vote_test;

/// Handle raft vote related operations
///
/// A `vote` defines the state of a openraft node.
/// See [`RaftState::calc_server_state`] .
pub(crate) struct VoteHandler<'st, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'st EngineConfig<C::NodeId>,
    pub(crate) state: &'st mut RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
    pub(crate) output: &'st mut EngineOutput<C>,
    pub(crate) internal_server_state:
        &'st mut InternalServerState<C::NodeId, <C::AsyncRuntime as AsyncRuntime>::Instant>,
}

impl<'st, C> VoteHandler<'st, C>
where C: RaftTypeConfig
{
    /// Validate and accept the input `vote` and send result via `tx`.
    ///
    /// If the vote is not GE the local vote, it sends an caller defined response via `tx` and
    /// returns `None` to inform the caller about the invalid vote.
    ///
    /// Otherwise it returns `Some(tx)`.
    ///
    /// The `f` is used to create the error response.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn accept_vote<T, E, F>(
        &mut self,
        vote: &Vote<C::NodeId>,
        tx: ResultSender<T, E>,
        f: F,
    ) -> Option<ResultSender<T, E>>
    where
        T: Debug + Eq,
        E: Debug + Eq,
        Respond<C::NodeId, C::Node>: From<ValueSender<Result<T, E>>>,
        F: Fn(
            &RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
            RejectVoteRequest<C::NodeId>,
        ) -> Result<T, E>,
    {
        let vote_res = self.update_vote(vote);

        if let Err(e) = vote_res {
            let res = f(self.state, e);

            self.output.push_command(Command::Respond {
                when: None,
                resp: Respond::new(res, tx),
            });

            return None;
        }
        Some(tx)
    }

    /// Check and update the local vote and related state for every message received.
    ///
    /// This is used by all incoming event, such as the three RPC append-entries, vote,
    /// install-snapshot to check the `vote` field.
    ///
    /// It grants the input vote and persists it if `input_vote >= my_vote`.
    ///
    /// Note: This method does not check last-log-id. handle-vote-request has to deal with
    /// last-log-id itself.
    pub(crate) fn update_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), RejectVoteRequest<C::NodeId>> {
        // Partial ord compare:
        // Vote does not has to be total ord.
        // `!(a >= b)` does not imply `a < b`.
        if vote >= self.state.vote_ref() {
            // Ok
        } else {
            tracing::info!("vote {} is rejected by local vote: {}", vote, self.state.vote_ref());
            return Err(RejectVoteRequest::ByVote(*self.state.vote_ref()));
        }
        tracing::debug!(%vote, "vote is changing to" );

        // Grant the vote

        if vote > self.state.vote_ref() {
            tracing::info!("vote is changing from {} to {}", self.state.vote_ref(), vote);

            self.state.vote.update(<C::AsyncRuntime as AsyncRuntime>::Instant::now(), *vote);
            self.output.push_command(Command::SaveVote { vote: *vote });
        } else {
            self.state.vote.touch(<C::AsyncRuntime as AsyncRuntime>::Instant::now());
        }

        // Update vote related timer and lease.

        tracing::debug!(
            now = debug(<C::AsyncRuntime as AsyncRuntime>::Instant::now()),
            "{}",
            func_name!()
        );

        self.update_internal_server_state();

        Ok(())
    }

    /// Enter leading or following state by checking `vote`.
    pub(crate) fn update_internal_server_state(&mut self) {
        if self.state.is_leading(&self.config.id) {
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
                l.vote = UTime::without_utime(*self.state.vote_ref());
                self.server_state_handler().update_server_state_if_changed();
                return;
            }
        }

        // It's a different leader that creates this vote.
        // Re-create a new Leader instance.

        let em = &self.state.membership_state.effective();
        let leader = Leading::new(
            *self.state.vote_ref(),
            em.membership().to_quorum_set(),
            em.learner_ids(),
            self.state.last_log_id().copied(),
        );

        // Do not update clock_progress, until the first blank log is committed.

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

    pub(crate) fn server_state_handler(&mut self) -> ServerStateHandler<C> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}
