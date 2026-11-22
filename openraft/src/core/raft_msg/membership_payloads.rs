use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::entry::RaftPayload;
use crate::type_config::alias::OneshotSenderOf;

/// The base payloads a membership change offers for the entries `RaftCore` writes.
///
/// A change writes either one entry, holding the target membership that its edit computes, or two:
/// an intermediate joint entry first, and the target once the joint entry commits. The caller
/// cannot know in advance which case applies, so it hands over one payload per entry and lets
/// `RaftCore` select.
pub(crate) enum MembershipPayloads<C>
where C: RaftTypeConfig
{
    /// The change writes one entry, its target, so one payload covers it.
    ///
    /// Adding a learner changes no voter set, so it never needs an intermediate entry. Its target
    /// may still hold several voter configs.
    Uniform(C::Payload),

    /// The change may write an intermediate joint entry before its target.
    ///
    /// `joint` is for the intermediate entry, and `uniform` for the entry that holds the exact
    /// target, whatever the target's shape: a custom edit may return a joint target, which a
    /// direct step writes with `uniform`. When `RaftCore` writes the intermediate entry, it sends
    /// `uniform`, bound to the target, back through `unused_tx`, and the caller appends it after
    /// the joint entry commits. For a direct step it sends `None`, so the caller knows that the
    /// change is complete.
    JointOrUniform {
        joint: C::Payload,
        uniform: C::Payload,
        unused_tx: OneshotSenderOf<C, Option<C::Payload>>,
    },
}

impl<C> MembershipPayloads<C>
where C: RaftTypeConfig
{
    /// Bind `membership` into the payload for the entry that holds it, and return that payload.
    ///
    /// `target` is the exact membership to append after an intermediate joint entry, or `None`
    /// when `membership` is the target itself.
    pub(crate) fn select(self, membership: C::Membership, target: Option<C::Membership>) -> C::Payload {
        let payload = match self {
            Self::Uniform(uniform) => {
                // Only `add_learner()` uses this variant, and it moves no voter, so no second step
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
