use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::raft::Precondition;
use crate::type_config::alias::BatchOf;
use crate::type_config::alias::MembershipEditOf;

/// Parameters for a membership change that may carry an application-defined payload.
#[since(version = "0.10.0", change = "added configurable membership change request")]
pub struct ChangeMembershipRequest<C>
where C: RaftTypeConfig
{
    members: MembershipEditOf<C>,
    retain_removed_as_learners: bool,
    preconditions: BatchOf<C, Precondition<C>>,
    payload: Option<(C::Payload, C::Payload)>,
}

impl<C> ChangeMembershipRequest<C>
where C: RaftTypeConfig
{
    /// Create a request that uses a new blank payload for each membership entry and has no
    /// preconditions.
    #[since(
        version = "0.10.0",
        change = "take `impl Into<MembershipEditOf<C>>` instead of `impl Into<ChangeMembers>`"
    )]
    #[since(version = "0.10.0", change = "added configurable membership change request")]
    pub fn new(members: impl Into<MembershipEditOf<C>>, retain: bool) -> Self {
        let members = members.into();
        Self {
            members,
            retain_removed_as_learners: retain,
            preconditions: BatchOf::<C, _>::of([]),
            payload: None,
        }
    }

    /// Use separate application-defined payloads for the membership-change steps.
    ///
    /// Each payload is tied to an entry of the change, not to the shape of its membership.
    /// `joint_payload` is used only for an intermediate joint entry, which the change writes only
    /// when openraft cannot prove that every quorum of the current membership intersects every
    /// quorum of the target. `uniform_payload` is used for the entry that holds the target, which
    /// every completed change writes, even when a custom edit returns a joint target. A change
    /// that needs no intermediate entry therefore drops `joint_payload` and writes its single entry
    /// with `uniform_payload`.
    #[since(version = "0.10.0", change = "select each payload by the shape of its membership")]
    #[since(version = "0.10.0", change = "accept separate payloads for membership change steps")]
    #[since(version = "0.10.0", change = "added optional membership change payload")]
    pub fn with_payload(mut self, joint_payload: C::Payload, uniform_payload: C::Payload) -> Self {
        self.payload = Some((joint_payload, uniform_payload));
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
        MembershipEditOf<C>,
        bool,
        BatchOf<C, Precondition<C>>,
        Option<(C::Payload, C::Payload)>,
    ) {
        (
            self.members,
            self.retain_removed_as_learners,
            self.preconditions,
            self.payload,
        )
    }
}
