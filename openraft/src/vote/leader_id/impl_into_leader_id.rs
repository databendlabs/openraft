use crate::node::NodeId;
use crate::vote::leader_id::CommittedLeaderId;
use crate::vote::Vote;

impl<NID: NodeId> From<Vote<NID>> for CommittedLeaderId<NID> {
    fn from(vote: Vote<NID>) -> Self {
        vote.leader_id.to_committed()
    }
}
