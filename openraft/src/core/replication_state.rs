use std::fmt::Debug;
use std::fmt::Formatter;

use crate::raft_types::LogIdOptionExt;
use crate::replication::ReplicationStream;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
pub(crate) struct ReplicationState<NID: NodeId> {
    pub matched: Option<LogId<NID>>,

    pub remove_since: Option<u64>,

    pub repl_stream: ReplicationStream<NID>,

    /// Count of replication failures.
    ///
    /// It will be reset once a successful replication is done.
    pub failures: u64,
}

impl<NID: NodeId> MessageSummary<ReplicationState<NID>> for ReplicationState<NID> {
    fn summary(&self) -> String {
        format!(
            "matched: {:?}, remove_after_commit: {:?}, failures: {}",
            self.matched, self.remove_since, self.failures
        )
    }
}

impl<NID: NodeId> Debug for ReplicationState<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplicationState")
            .field("matched", &self.matched)
            .field("remove_since", &self.remove_since)
            .field("failures", &self.failures)
            .finish()
    }
}

/// Calculate the distance between the matched log id on a replication target and local last log id
pub(crate) fn replication_lag<NID: NodeId>(matched: &Option<LogId<NID>>, last_log_id: &Option<LogId<NID>>) -> u64 {
    last_log_id.next_index().saturating_sub(matched.next_index())
}

#[cfg(test)]
mod test {
    use crate::core::replication_state::replication_lag;
    use crate::LeaderId;
    use crate::LogId;

    #[test]
    fn test_replication_lag() -> anyhow::Result<()> {
        let log_id = |term, node_id, index| LogId::<u64>::new(LeaderId::new(term, node_id), index);

        assert_eq!(0, replication_lag::<u64>(&None, &None));
        assert_eq!(4, replication_lag::<u64>(&None, &Some(log_id(1, 2, 3))));
        assert_eq!(
            1,
            replication_lag::<u64>(&Some(log_id(1, 2, 2)), &Some(log_id(1, 2, 3)))
        );
        assert_eq!(
            0,
            replication_lag::<u64>(&Some(log_id(1, 2, 3)), &Some(log_id(1, 2, 3)))
        );
        assert_eq!(
            0,
            replication_lag::<u64>(&Some(log_id(1, 2, 4)), &Some(log_id(1, 2, 3)))
        );
        Ok(())
    }
}
