use crate::Membership;
use crate::RaftTypeConfig;
/// Defines operations on an entry payload.
pub trait RaftPayload<C>
where C: RaftTypeConfig
{
    /// Return `Some(Membership)` if the entry payload contains a membership payload.
    fn get_membership(&self) -> Option<Membership<C>>;
}
