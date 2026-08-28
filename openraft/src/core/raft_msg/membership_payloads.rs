use crate::Membership;
use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::entry::RaftPayload;
use crate::type_config::alias::OneshotSenderOf;

/// The base payloads a membership change offers for the entry `RaftCore` computes.
///
/// A change proposes one entry, and `RaftCore` decides the shape of its membership. The caller
/// therefore cannot know in advance which entry its payload lands on, so it hands over one
/// payload per shape and lets `RaftCore` select.
pub(crate) enum MembershipPayloads<C>
where C: RaftTypeConfig
{
    /// The change cannot compute a joint membership, so one payload covers its only entry.
    ///
    /// Adding a learner and flattening a joint membership both leave the voter sets alone.
    Uniform(C::Payload),

    /// The change may compute a joint or a uniform membership.
    ///
    /// `RaftCore` selects the payload of the computed shape and sends the other one back through
    /// `unused_tx`. A joint membership therefore returns `uniform`, which the caller spends on
    /// the entry that flattens that joint membership.
    JointOrUniform {
        joint: C::Payload,
        uniform: C::Payload,
        unused_tx: OneshotSenderOf<C, C::Payload>,
    },
}

impl<C> MembershipPayloads<C>
where C: RaftTypeConfig
{
    /// Bind `membership` into the payload that matches its shape, and return that payload.
    pub(crate) fn select(self, membership: Membership<C::NodeId, C::Node>) -> C::Payload {
        let payload = match self {
            Self::Uniform(uniform) => uniform,
            Self::JointOrUniform {
                joint,
                uniform,
                unused_tx,
            } => {
                let (selected, unused) = if membership.is_joint() {
                    (joint, uniform)
                } else {
                    (uniform, joint)
                };

                // The caller owns the payload this entry does not spend. It needs the uniform
                // one back to write the entry that flattens a joint membership.
                let _ = unused_tx.send(unused);
                selected
            }
        };

        payload.with_membership(membership)
    }
}
