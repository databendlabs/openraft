use openraft_macros::since;

use crate::ChangeMembers;
use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::raft::Precondition;
use crate::type_config::alias::BatchOf;

/// Parameters for a membership change that may carry an application-defined payload.
#[since(version = "0.10.0", change = "added configurable membership change request")]
pub struct ChangeMembershipRequest<C>
where C: RaftTypeConfig
{
    members: ChangeMembers<C::NodeId, C::Node>,
    retain: bool,
    preconditions: BatchOf<C, Precondition<C>>,
    payload: Option<(C::Payload, C::Payload)>,
}

impl<C> ChangeMembershipRequest<C>
where C: RaftTypeConfig
{
    /// Create a request without an application-defined payload or preconditions.
    #[since(version = "0.10.0", change = "added configurable membership change request")]
    pub fn new(members: impl Into<ChangeMembers<C::NodeId, C::Node>>, retain: bool) -> Self {
        let members = members.into();
        Self {
            members,
            retain,
            preconditions: BatchOf::<C, _>::of([]),
            payload: None,
        }
    }

    /// Use the same application-defined payload for both steps of a voter change.
    #[since(version = "0.10.0", change = "added optional membership change payload")]
    pub fn with_payload(mut self, payload: C::Payload) -> Self
    where C::Payload: Clone {
        let first_payload = payload.clone();
        self.payload = Some((first_payload, payload));
        self
    }

    /// Guard the first membership proposal with the given preconditions.
    #[since(version = "0.10.0", change = "added membership change preconditions")]
    pub fn with_preconditions(mut self, preconditions: impl IntoIterator<Item = Precondition<C>>) -> Self {
        let preconditions = BatchOf::<C, _>::of(preconditions);
        self.preconditions = preconditions;
        self
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ChangeMembers<C::NodeId, C::Node>,
        bool,
        BatchOf<C, Precondition<C>>,
        Option<(C::Payload, C::Payload)>,
    ) {
        (self.members, self.retain, self.preconditions, self.payload)
    }
}
