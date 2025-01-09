use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::RaftTypeConfig;

/// Defines API to operate an object that contains a log-id, such as a log entry or a log id.
pub trait RaftLogId<C>
where C: RaftTypeConfig
{
    /// Returns a reference to the leader id that proposed this log id.  
    ///
    /// When a `LeaderId` is committed, some of its data can be discarded.
    /// For example, a leader id in standard raft is `(term, node_id)`, but a log id does not have
    /// to store the `node_id`, because in standard raft there is at most one leader that can be
    /// established.
    fn leader_id(&self) -> &CommittedLeaderIdOf<C> {
        self.get_log_id().committed_leader_id()
    }

    /// Return a reference to the log-id it stores.
    fn get_log_id(&self) -> &LogIdOf<C>;

    /// Update the log id it contains.
    fn set_log_id(&mut self, log_id: &LogIdOf<C>);
}
