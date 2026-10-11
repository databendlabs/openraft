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
    /// `RaftCore` selects the payload of the computed shape. For a joint membership it sends
    /// `uniform` back through `unused_tx`, and the caller spends it on the entry that flattens
    /// that joint membership. For a uniform membership it sends `None`, so the caller knows that
    /// the change is complete.
    JointOrUniform {
        joint: C::Payload,
        uniform: C::Payload,
        unused_tx: OneshotSenderOf<C, Option<C::Payload>>,
    },
}

impl<C> MembershipPayloads<C>
where C: RaftTypeConfig
{
    /// Bind `membership` into the payload that matches its shape, and return that payload.
    ///
    /// `finished` is true when `membership` is the target of the change, and false when it is a
    /// joint membership.
    pub(crate) fn select(self, membership: C::Membership, finished: bool) -> C::Payload {
        let payload = match self {
            Self::Uniform(uniform) => uniform,
            Self::JointOrUniform {
                joint,
                uniform,
                unused_tx,
            } => {
                let (selected, unused) = if finished {
                    (uniform, None)
                } else {
                    (joint, Some(uniform))
                };

                // The caller needs the uniform payload back only after a joint membership, to
                // write the entry that flattens it.
                let _ = unused_tx.send(unused);
                selected
            }
        };

        payload.with_membership(membership)
    }
}
