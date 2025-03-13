use std::fmt::Debug;

use crate::core::raft_msg::ResultSender;
use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::Respond;
use crate::engine::ValueSender;
use crate::entry::RaftEntry;
use crate::error::RejectVoteRequest;
use crate::proposer::CandidateState;
use crate::proposer::LeaderState;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::InstantOf;
use crate::type_config::TypeConfigExt;
use crate::AsyncRuntime;
use crate::LogId;
use crate::OptionalSend;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::Vote;

#[cfg(test)]
mod accept_vote_test;
#[cfg(test)]
mod handle_message_vote_test;

/// Handle raft vote related operations
///
/// A `vote` defines the state of a openraft node.
/// See [`RaftState::calc_server_state`] .
pub(crate) struct VoteHandler<'st, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'st mut EngineConfig<C::NodeId>,
    pub(crate) state: &'st mut RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
    pub(crate) output: &'st mut EngineOutput<C>,
    pub(crate) leader: &'st mut LeaderState<C>,
    pub(crate) candidate: &'st mut CandidateState<C>,
}

impl<C> VoteHandler<'_, C>
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
        tx: ResultSender<C, T, E>,
        f: F,
    ) -> Option<ResultSender<C, T, E>>
    where
        T: Debug + Eq + OptionalSend,
        E: Debug + Eq + OptionalSend,
        Respond<C>: From<ValueSender<C, Result<T, E>>>,
        F: Fn(&RaftState<C::NodeId, C::Node, InstantOf<C>>, RejectVoteRequest<C::NodeId>) -> Result<T, E>,
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
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), RejectVoteRequest<C::NodeId>> {
        // Partial ord compare:
        // Vote does not has to be total ord.
        // `!(a >= b)` does not imply `a < b`.
        if vote >= self.state.vote_ref() {
            // Ok
        } else {
            tracing::info!("vote {} is rejected by local vote: {}", vote, self.state.vote_ref());
            return Err(RejectVoteRequest::ByVote(self.state.vote_ref().clone()));
        }
        tracing::debug!(%vote, "vote is changing to" );

        // Grant the vote

        if vote > self.state.vote_ref() {
            tracing::info!("vote is changing from {} to {}", self.state.vote_ref(), vote);

            self.state.vote.update(C::now(), vote.clone());
            self.output.push_command(Command::SaveVote { vote: vote.clone() });
        } else {
            self.state.vote.touch(C::now());
        }

        // Update vote related timer and lease.

        tracing::debug!(now = debug(C::now()), "{}", func_name!());

        self.update_internal_server_state();

        Ok(())
    }

    /// Update to Leader or following state depending on `Engine.state.vote`.
    pub(crate) fn update_internal_server_state(&mut self) {
        if self.state.is_leader(&self.config.id) {
            self.become_leader();
        } else if self.state.is_leading(&self.config.id) {
            // candidate, nothing to do
        } else {
            self.become_following();
        }
    }

    /// Enter Leader state(vote.node_id == self.id && vote.committed == true) .
    ///
    /// Note that this is called when the Leader state changes caused by
    /// the change of vote in the **Acceptor** part `engine.state.vote`.
    /// This is **NOT** called when a node is elected as a leader.
    ///
    /// An example use of this mechanism is when node-a electing for node-b,
    /// which can be used for Leader-a to transfer leadership to Follower-b.
    pub(crate) fn become_leader(&mut self) {
        tracing::debug!(
            "become leader: node-{}, my vote: {}, last-log-id: {}",
            self.config.id,
            self.state.vote_ref(),
            self.state.last_log_id().cloned().unwrap_or_default()
        );

        if let Some(l) = self.leader.as_mut() {
            tracing::debug!("leading vote: {}", l.vote,);

            if l.vote.leader_id() == self.state.vote_ref().leader_id() {
                tracing::debug!(
                    "vote still belongs to the same leader. Just updating vote is enough: node-{}, {}",
                    self.config.id,
                    self.state.vote_ref()
                );
                // TODO: this is not gonna happen,
                //       because `self.leader`(previous `internal_server_state`)
                //       does not include Candidate any more.
                l.vote = self.state.vote_ref().clone();
                self.server_state_handler().update_server_state_if_changed();
                return;
            }
        }

        // It's a different leader that creates this vote.
        // Re-create a new Leader instance.

        let leader = self.state.new_leader();
        *self.leader = Some(Box::new(leader));

        self.server_state_handler().update_server_state_if_changed();

        self.replication_handler().rebuild_replication_streams();

        let leader = self.leader.as_ref().unwrap();

        // If the leader has not yet proposed any log, propose a blank log
        if leader.last_log_id() < leader.noop_log_id() {
            self.leader_handler()
                .leader_append_entries(vec![C::Entry::new_blank(LogId::<C::NodeId>::default())]);
        }
    }

    /// Enter following state(vote.node_id != self.id or self is not a voter).
    ///
    /// This node then becomes raft-follower or raft-learner.
    pub(crate) fn become_following(&mut self) {
        // TODO: entering following needs to check last-log-id on other node to decide the election
        // timeout.

        debug_assert!(
            self.state.vote_ref().leader_id().voted_for() != Some(self.config.id.clone())
                || !self.state.membership_state.effective().membership().is_voter(&self.config.id),
            "It must hold: vote is not mine, or I am not a voter(leader just left the cluster)"
        );

        *self.leader = None;
        *self.candidate = None;

        self.server_state_handler().update_server_state_if_changed();
    }

    pub(crate) fn server_state_handler(&mut self) -> ServerStateHandler<C> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<C> {
        let leader = self.leader.as_mut().unwrap();

        ReplicationHandler {
            config: self.config,
            leader,
            state: self.state,
            output: self.output,
        }
    }

    pub(crate) fn leader_handler(&mut self) -> LeaderHandler<C> {
        let leader = self.leader.as_mut().unwrap();

        LeaderHandler {
            config: self.config,
            leader,
            state: self.state,
            output: self.output,
        }
    }
}
