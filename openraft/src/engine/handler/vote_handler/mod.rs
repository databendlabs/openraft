use std::fmt::Debug;

use crate::core::raft_msg::ResultSender;
use crate::display_ext::DisplayInstantExt;
use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::replication_handler::SendNone;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::Respond;
use crate::engine::ValueSender;
use crate::entry::RaftEntry;
use crate::error::RejectVoteRequest;
use crate::proposer::CandidateState;
use crate::proposer::LeaderState;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::type_config::TypeConfigExt;
use crate::LogId;
use crate::OptionalSend;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::Vote;

#[cfg(test)]
mod accept_vote_test;
#[cfg(test)]
mod become_leader_test;
#[cfg(test)]
mod handle_message_vote_test;

/// Handle raft vote related operations
///
/// A `vote` defines the state of a openraft node.
/// See [`RaftState::calc_server_state`] .
pub(crate) struct VoteHandler<'st, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'st mut EngineConfig<C>,
    pub(crate) state: &'st mut RaftState<C>,
    pub(crate) output: &'st mut EngineOutput<C>,
    pub(crate) last_seen_vote: &'st mut Vote<C::NodeId>,
    pub(crate) leader: &'st mut LeaderState<C>,
    pub(crate) candidate: &'st mut CandidateState<C>,
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
        tx: ResultSender<C, T, E>,
        f: F,
    ) -> Option<ResultSender<C, T, E>>
    where
        T: Debug + Eq + OptionalSend,
        E: Debug + Eq + OptionalSend,
        Respond<C>: From<ValueSender<C, Result<T, E>>>,
        F: Fn(&RaftState<C>, RejectVoteRequest<C>) -> Result<T, E>,
    {
        let vote_res = self.update_vote(vote);

        if let Err(e) = vote_res {
            let res = f(self.state, e);

            let condition = Some(Condition::IOFlushed {
                io_id: IOId::new(*self.state.vote_ref()),
            });

            self.output.push_command(Command::Respond {
                when: condition,
                resp: Respond::new(res, tx),
            });

            return None;
        }
        Some(tx)
    }

    /// Update the `last_seen_vote` to a greater value.
    ///
    /// Return the replaced value if it is updated.
    /// Return None if not updated.
    pub(crate) fn update_last_seen(&mut self, vote: &Vote<C::NodeId>) -> Option<Vote<C::NodeId>> {
        tracing::debug!(
            "about to update last_seen_vote from {} to {}",
            self.last_seen_vote,
            vote
        );

        if vote >= self.last_seen_vote {
            tracing::info!("updated last_seen_vote from {} to {}", self.last_seen_vote, vote);
            let last = *self.last_seen_vote;
            *self.last_seen_vote = *vote;
            Some(last)
        } else {
            None
        }
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
    ///
    /// This method also implies calling [`Self::update_last_seen`].
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), RejectVoteRequest<C>> {
        self.update_last_seen(vote);

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

            self.state.vote.update(C::now(), *vote);
            self.state.accept_io(IOId::new(*vote));
            self.output.push_command(Command::SaveVote { vote: *vote });
        } else {
            self.state.vote.touch(C::now());
        }

        // Update vote related timer and lease.

        tracing::debug!(now = display(C::now().display()), "{}", func_name!());

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
            self.state.last_log_id().copied().unwrap_or_default()
        );

        if let Some(l) = self.leader.as_mut() {
            tracing::debug!("leading vote: {}", l.committed_vote,);

            if l.committed_vote.leader_id() == self.state.vote_ref().leader_id() {
                tracing::debug!(
                    "vote still belongs to the same leader. Just updating vote is enough: node-{}, {}",
                    self.config.id,
                    self.state.vote_ref()
                );
                // TODO: this is not gonna happen,
                //       because `self.leader`(previous `internal_server_state`)
                //       does not include Candidate any more.
                l.committed_vote = self.state.vote_ref().into_committed();
                self.server_state_handler().update_server_state_if_changed();
                return;
            }
        }

        // It's a different leader that creates this vote.
        // Re-create a new Leader instance.

        let leader = self.state.new_leader();
        let leader_vote = *leader.committed_vote_ref();
        *self.leader = Some(Box::new(leader));

        let (last_log_id, noop_log_id) = {
            let leader = self.leader.as_ref().unwrap();
            (leader.last_log_id().copied(), leader.noop_log_id().copied())
        };

        self.state.accept_io(IOId::new_log_io(leader_vote, last_log_id));

        self.server_state_handler().update_server_state_if_changed();

        let mut rh = self.replication_handler();
        rh.rebuild_replication_streams();

        // If the leader has not yet proposed any log, propose a blank log and initiate replication;
        // Otherwise, just initiate replication.
        if last_log_id < noop_log_id {
            self.leader_handler()
                .leader_append_entries(vec![C::Entry::new_blank(LogId::<C::NodeId>::default())]);
        } else {
            self.replication_handler().initiate_replication(SendNone::False);
        }
    }

    /// Enter following state(vote.node_id != self.id or self is not a voter).
    ///
    /// This node then becomes raft-follower or raft-learner.
    pub(crate) fn become_following(&mut self) {
        debug_assert!(
            self.state.vote_ref().leader_id().voted_for() != Some(self.config.id)
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
