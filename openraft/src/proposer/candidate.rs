use std::fmt;
use std::sync::Arc;

use display_more::DisplayOptionExt;
use validit::Valid;

use crate::RaftTypeConfig;
use crate::base::shared_id_generator::SharedIdGenerator;
use crate::display_ext::DisplayInstantExt;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::membership::RaftMembership;
use crate::progress::IdVal;
use crate::progress::VecProgress;
use crate::proposer::ElectionQuorumSet;
use crate::proposer::Leader;
use crate::proposer::WriteQuorumSet;
use crate::quorum::BothQuorums;
use crate::quorum::QuorumSet;
use crate::quorum::ReadOrBoth;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// Candidate: voting state.
#[derive(Clone, Debug)]
pub(crate) struct Candidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<Id = C::NodeId>,
{
    /// When the voting is started.
    starting_time: InstantOf<C>,

    /// The vote.
    vote: VoteOf<C>,

    last_log_id: Option<LogIdOf<C>>,

    /// Which nodes have granted the vote at certain time point.
    progress: Valid<VecProgress<IdVal<C::NodeId, bool>, QS>>,

    membership: C::Membership,

    progress_id_gen: SharedIdGenerator,
}

impl<C, QS> fmt::Display for Candidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<Id = C::NodeId> + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{}@{}, last_log_id:{} progress:{}}}",
            self.vote,
            self.starting_time.display(),
            self.last_log_id.display(),
            self.progress
        )
    }
}

impl<C> Candidate<C, ElectionQuorumSet<C>>
where C: RaftTypeConfig
{
    /// Creates a candidate that wins by the election rule of `C::LeaderId`.
    ///
    /// Totally ordered leader IDs need only a read quorum; other leader IDs need a vote set that is
    /// both a read and a write quorum.
    pub(crate) fn new(
        starting_time: InstantOf<C>,
        vote: VoteOf<C>,
        last_log_id: Option<LogIdOf<C>>,
        membership: C::Membership,
        progress_id_gen: SharedIdGenerator,
    ) -> Self {
        let read_quorum_set = membership.read_quorum_set();
        let election_quorum_set = if C::LeaderId::TOTALLY_ORDERED {
            ReadOrBoth::Read(Arc::new(read_quorum_set))
        } else {
            let write_quorum_set = membership.write_quorum_set();
            let both = BothQuorums::new(Arc::new(read_quorum_set), Arc::new(write_quorum_set));
            ReadOrBoth::Both(both)
        };
        Self {
            starting_time,
            vote,
            last_log_id,
            progress: Valid::new(VecProgress::new(election_quorum_set, [], IdVal::new_default)),
            membership,
            progress_id_gen,
        }
    }
}

impl<C, QS> Candidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<Id = C::NodeId> + fmt::Debug + Clone + 'static,
{
    pub(crate) fn vote_ref(&self) -> &VoteOf<C> {
        &self.vote
    }

    pub(crate) fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.last_log_id.as_ref()
    }

    /// The time at which this voting round started.
    ///
    /// Used to rate-limit Pre-Vote retries: a fresh round already in flight should not be restarted
    /// on every tick.
    pub(crate) fn starting_time(&self) -> InstantOf<C> {
        self.starting_time
    }

    pub(crate) fn progress(&self) -> &VecProgress<IdVal<C::NodeId, bool>, QS> {
        &self.progress
    }

    /// Grant the vote by a node.
    pub(crate) fn grant_by(&mut self, target: &C::NodeId) -> bool {
        let Some(granted) = self.progress.update_progress(target, true) else {
            tracing::warn!(
                "{}: ignore vote from target not in quorum set: {}",
                func_name!(),
                target
            );
            return false;
        };
        let granted = *granted;

        tracing::info!("{}: voting: {}", func_name!(), self);

        granted
    }

    /// Return the node ids that have granted this vote.
    #[allow(dead_code)]
    pub(crate) fn granters(&self) -> impl Iterator<Item = C::NodeId> + '_ {
        self.progress().iter().filter(|item| item.val).map(|item| item.id.clone())
    }
}

impl<C> Candidate<C, ElectionQuorumSet<C>>
where C: RaftTypeConfig
{
    pub(crate) fn into_leader(self) -> Leader<C, WriteQuorumSet<C>> {
        // Mark the vote as committed, i.e., being granted and saved by a quorum.
        let vote = {
            let vote = self.vote_ref().clone();
            debug_assert!(!vote.is_committed());
            vote.to_committed()
        };

        // TODO: tricky: the new LeaderId is different from the last log id
        //       Thus only the last().index is used.
        //       Thus the first() is ignored.
        //       But we should not fake the first() there.
        let last = self.last_log_id();
        let last_leader_log_ids = last.cloned().map(LeaderLogIds::new_single);

        Leader::new(
            vote,
            self.membership,
            last_leader_log_ids, // already Option<LeaderLogIds>
            self.progress_id_gen,
        )
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreeset;
    use openraft_rt_tokio::TokioRuntime;

    use crate::Membership;
    use crate::Vote;
    use crate::base::shared_id_generator::SharedIdGenerator;
    use crate::declare_raft_types;
    use crate::engine::testing::UTConfig;
    use crate::proposer::Candidate;
    use crate::quorum::ReadOrBoth;
    use crate::type_config::TypeConfigExt;

    declare_raft_types!(
        StdConfig:
            LeaderId = crate::impls::leader_id_std::LeaderId<u64, u64>,
            AsyncRuntime = TokioRuntime,
    );

    #[test]
    fn test_new_with_advanced_leader_id_elects_by_read_quorum() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1,2,3}], []);
        let now = UTConfig::<()>::now();
        let vote = Vote::new(2, 1);
        let candidate = Candidate::<UTConfig, _>::new(now, vote, None, membership, SharedIdGenerator::new());

        let is_read = matches!(candidate.progress().quorum_set(), ReadOrBoth::Read(_));
        assert!(is_read, "advanced leader IDs are totally ordered");
    }

    #[test]
    fn test_new_with_standard_leader_id_elects_by_read_and_write_quorum() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1,2,3}], []);
        let now = StdConfig::now();
        let vote = Vote::new(2, 1);
        let candidate = Candidate::<StdConfig, _>::new(now, vote, None, membership, SharedIdGenerator::new());

        let is_read_and_write = matches!(candidate.progress().quorum_set(), ReadOrBoth::Both(_));
        assert!(is_read_and_write, "standard leader IDs are partially ordered");
    }
}
