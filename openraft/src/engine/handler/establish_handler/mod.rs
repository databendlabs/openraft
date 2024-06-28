use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::vote_handler::VoteHandler;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::entry::RaftEntry;
use crate::internal_server_state::InternalServerState;
use crate::internal_server_state::LeaderQuorumSet;
use crate::leader::candidate::Candidate;
use crate::LogId;
use crate::RaftState;
use crate::RaftTypeConfig;

/// Establish a leader for the Engine, when Candidate finishes voting stage.
pub(crate) struct EstablishHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut InternalServerState<C>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

impl<'x, C> EstablishHandler<'x, C>
where C: RaftTypeConfig
{
    /// Consume the `candidate` state and establish a leader.
    pub(crate) fn establish(&mut self, candidate: Candidate<C, LeaderQuorumSet<C::NodeId>>) {
        let vote = *candidate.vote_ref();

        debug_assert_eq!(
            vote.leader_id().voted_for(),
            Some(self.config.id),
            "it can only commit its own vote"
        );

        if let Some(l) = self.leader.leader_ref() {
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if !(&vote > l.vote_ref()) {
                tracing::warn!(
                    "vote is not greater than current existing leader vote. Do not establish new leader and quit"
                );
                return;
            }
        }

        let leader = candidate.into_leader();
        let vote = *leader.vote_ref();
        *self.leader = InternalServerState::Leader(Box::new(leader));

        self.replication_handler().rebuild_replication_streams();

        // Before sending any log, update the vote.
        // This could not fail because `internal_server_state` will be cleared
        // once `state.vote` is changed to a value of other node.
        let _res = self.vote_handler().update_vote(&vote);
        debug_assert!(_res.is_ok(), "commit vote can not fail but: {:?}", _res);

        self.leader_handler()
            .leader_append_entries(vec![C::Entry::new_blank(LogId::<C::NodeId>::default())]);
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<C> {
        let leader = self.leader.leader_mut().unwrap();

        ReplicationHandler {
            config: self.config,
            leader,
            state: self.state,
            output: self.output,
        }
    }

    pub(crate) fn vote_handler(&mut self) -> VoteHandler<C> {
        VoteHandler {
            config: self.config,
            state: self.state,
            output: self.output,
            leader: self.leader,
        }
    }

    pub(crate) fn leader_handler(&mut self) -> LeaderHandler<C> {
        let leader = self.leader.leader_mut().unwrap();

        LeaderHandler {
            config: self.config,
            leader,
            state: self.state,
            output: self.output,
        }
    }
}
