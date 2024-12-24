use crate::engine::EngineConfig;
use crate::proposer::Candidate;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::proposer::LeaderState;
use crate::vote::RaftLeaderId;
use crate::RaftTypeConfig;

/// Establish a leader for the Engine, when Candidate finishes voting stage.
pub(crate) struct EstablishHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut LeaderState<C>,
}

impl<'x, C> EstablishHandler<'x, C>
where C: RaftTypeConfig
{
    /// Consume the `candidate` state and establish a leader.
    pub(crate) fn establish(
        self,
        candidate: Candidate<C, LeaderQuorumSet<C>>,
    ) -> Option<&'x mut Leader<C, LeaderQuorumSet<C>>> {
        let vote = candidate.vote_ref().clone();

        debug_assert_eq!(
            vote.leader_id().node_id_ref(),
            Some(&self.config.id),
            "it can only commit its own vote"
        );

        if let Some(l) = self.leader.as_ref() {
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if !(vote > l.committed_vote_ref().clone().into_vote()) {
                tracing::warn!(
                    "vote is not greater than current existing leader vote. Do not establish new leader and quit"
                );
                return None;
            }
        }

        let leader = candidate.into_leader();
        *self.leader = Some(Box::new(leader));

        self.leader.as_mut().map(|x| x.as_mut())
    }
}
