use std::fmt;

use crate::Membership;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayOption;
use crate::type_config::alias::LogIdOf;

/// This struct represents information about a membership config that has already been stored in the
/// raft logs.
///
/// It includes log id and a membership config. Such a record is used in the state machine or
/// snapshot to track the last membership and its log id. And it is also used as a return value for
/// functions that return membership and its log position.
///
/// It derives `Default` for building an uninitialized membership state, e.g., when a raft-node is
/// just created.
#[derive(Clone, Debug, Default)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct StoredMembership<C>
where C: RaftTypeConfig
{
    /// The id of the log that stores this membership config
    log_id: Option<LogIdOf<C>>,

    /// Membership config
    membership: Membership<C>,
}

impl<C> StoredMembership<C>
where C: RaftTypeConfig
{
    pub fn new(log_id: Option<LogIdOf<C>>, membership: Membership<C>) -> Self {
        Self { log_id, membership }
    }

    pub fn log_id(&self) -> &Option<LogIdOf<C>> {
        &self.log_id
    }

    pub fn membership(&self) -> &Membership<C> {
        &self.membership
    }

    pub fn voter_ids(&self) -> impl Iterator<Item = C::NodeId> {
        self.membership.voter_ids()
    }

    pub fn nodes(&self) -> impl Iterator<Item = (&C::NodeId, &C::Node)> {
        self.membership.nodes()
    }
}

impl<C> fmt::Display for StoredMembership<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{log_id:{}, {}}}", DisplayOption(&self.log_id), self.membership)
    }
}
