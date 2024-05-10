use crate::node::NodeId;
use crate::vote::leader_id::LeaderId;
use crate::vote::Vote;

impl<NID: NodeId> From<Vote<NID>> for LeaderId<NID> {
    fn from(vote: Vote<NID>) -> Self {
        vote.leader_id
    }
}
