use crate::raft_types::LogIdOptionExt;
use crate::LogId;
use crate::NodeId;

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
