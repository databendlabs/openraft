use crate::CommittedLeaderId;
use crate::LogId;
use crate::NodeId;

/// Defines API to operate an object that contains a log-id, such as a log entry or a log id.
pub trait RaftLogId<NID: NodeId> {
    /// Returns a reference to the leader id that proposed this log id.  
    ///
    /// When a `LeaderId` is committed, some of its data can be discarded.
    /// For example, a leader id in standard raft is `(term, node_id)`, but a log id does not have
    /// to store the `node_id`, because in standard raft there is at most one leader that can be
    /// established.
    fn leader_id(&self) -> &CommittedLeaderId<NID> {
        self.get_log_id().committed_leader_id()
    }

    /// Return a reference to the log-id it stores.
    fn get_log_id(&self) -> &LogId<NID>;

    /// Update the log id it contains.
    fn set_log_id(&mut self, log_id: &LogId<NID>);
}
