use crate::raft_types::LogIndexOptionExt;

/// Calculate the distance between the matched log index on a replication target and local last log index
pub(crate) fn replication_lag(matched_log_index: &Option<u64>, last_log_index: &Option<u64>) -> u64 {
    last_log_index.next_index().saturating_sub(matched_log_index.next_index())
}

#[cfg(test)]
mod test {
    use crate::core::replication_state::replication_lag;

    #[test]
    fn test_replication_lag() -> anyhow::Result<()> {
        assert_eq!(0, replication_lag(&None, &None));
        assert_eq!(4, replication_lag(&None, &Some(3)));
        assert_eq!(1, replication_lag(&Some(2), &Some(3)));
        assert_eq!(0, replication_lag(&Some(3), &Some(3)));
        assert_eq!(0, replication_lag(&Some(4), &Some(3)));
        Ok(())
    }
}
