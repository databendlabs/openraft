use crate::vote::leader_id::CommittedLeaderId;
use crate::vote::Vote;
use crate::RaftTypeConfig;

impl<C> From<Vote<C>> for CommittedLeaderId<C::NodeId>
where C: RaftTypeConfig
{
    fn from(vote: Vote<C>) -> Self {
        vote.leader_id.to_committed()
    }
}
