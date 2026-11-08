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
    /// Adding a learner leaves the voter sets alone.
    Uniform(C::Payload),

    /// The change may compute a joint or a uniform membership.
    ///
    /// `RaftCore` selects the payload of the computed shape. For a joint membership it sends
    /// `uniform` with the exact target back through `unused_tx`, and the caller appends it after
    /// the joint membership commits. For a uniform membership it sends `None`, so the caller
    /// knows that the change is complete.
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
    /// `target` is the exact membership to append after a joint step, or `None` for a direct step.
    pub(crate) fn select(self, membership: C::Membership, target: Option<C::Membership>) -> C::Payload {
        let payload = match self {
            Self::Uniform(uniform) => {
                // Only `add_learner()` uses this shape, and it moves no voter, so no second step
                // follows and there is no target to bind.
                debug_assert!(target.is_none());
                uniform
            }
            Self::JointOrUniform {
                joint,
                uniform,
                unused_tx,
            } => {
                let (selected, unused) = match target {
                    Some(target) => {
                        // Bind the target before sending it back, so the caller appends the edit's
                        // node removals and roles rather than what the joint state implies.
                        let uniform = uniform.with_membership(target);
                        (joint, Some(uniform))
                    }
                    None => (uniform, None),
                };

                let _ = unused_tx.send(unused);
                selected
            }
        };

        payload.with_membership(membership)
    }
}
