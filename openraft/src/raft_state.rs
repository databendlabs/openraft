use std::sync::Arc;

use crate::membership::EffectiveMembership;
use crate::LogId;
use crate::NodeId;
use crate::ServerState;
use crate::Vote;

/// A struct used to represent the raft state which a Raft node needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaftState<NID: NodeId> {
    /// The vote state of this node.
    pub vote: Vote<NID>,

    /// The greatest log id that has been purged after being applied to state machine.
    /// The range of log entries that exist in storage is `(last_purged_log_id, last_log_id]`,
    /// left open and right close.
    ///
    /// `last_purged_log_id == last_log_id` means there is no log entry in the storage.
    pub last_purged_log_id: Option<LogId<NID>>,

    /// The id of the last log entry.
    pub last_log_id: Option<LogId<NID>>,

    /// The LogId of the last log applied to the state machine.
    pub last_applied: Option<LogId<NID>>,

    /// The latest cluster membership configuration found, in log or in state machine.
    pub effective_membership: Arc<EffectiveMembership<NID>>,

    // -- volatile fields: they are not persisted.
    /// The log id of the last known committed entry.
    ///
    /// - Committed means: a log that is replicated to a quorum of the cluster and it is of the term of the leader.
    ///
    /// - A quorum could be a uniform quorum or joint quorum.
    ///
    /// - `committed` in raft is volatile and will not be persisted.
    pub committed: Option<LogId<NID>>,

    pub server_state: ServerState,
}
