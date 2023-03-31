use std::fmt::Display;
use std::fmt::Formatter;

use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::Vote;

/// Uniquely identifies a replication session.
///
/// A replication session is a group of replication stream, e.g., the leader(node-1) in a cluster of
/// 3 nodes may have a replication config `{2,3}`.
///
/// Replication state can not be shared between two leaders(different `vote`) or between two
/// membership configs: E.g. given 3 membership log:
/// - `log_id=1, members={a,b,c}`
/// - `log_id=5, members={a,b}`
/// - `log_id=10, members={a,b,c}`
///
/// When log_id=1 is appended, openraft spawns a replication to node `c`.
/// Then log_id=1 is replicated to node `c`.
/// Then a replication state update message `{target=c, matched=log_id-1}` is piped in message
/// queue(`tx_api`), waiting the raft core to process.
///
/// Then log_id=5 is appended, replication to node `c` is dropped.
///
/// Then log_id=10 is appended, another replication to node `c` is spawned.
/// Now node `c` is a new empty node, no log is replicated to it.
/// But the delayed message `{target=c, matched=log_id-1}` may be process by raft core and make raft
/// core believe node `c` already has `log_id=1`, and commit it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplicationSessionId<NID: NodeId> {
    /// The vote of the leader.
    pub(crate) vote: Vote<NID>,

    /// The log id of the membership log this replication works for.
    pub(crate) membership_log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> Display for ReplicationSessionId<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.vote, self.membership_log_id.summary())
    }
}

impl<NID: NodeId> ReplicationSessionId<NID> {
    pub(crate) fn new(vote: Vote<NID>, membership_log_id: Option<LogId<NID>>) -> Self {
        Self {
            vote,
            membership_log_id,
        }
    }
}
