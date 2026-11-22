/// Calculate the distance between the matched log index on a replication target and the locally
/// known last log index.
///
/// It counts the logs after the matched one, up to and including the last one: 0 when the target
/// matched the last log or a later one. The only count that does not fit in `u64` is that of a log
/// holding every index, `0..=u64::MAX`, with nothing matched; it saturates at `u64::MAX`.
pub(crate) fn replication_lag(matched_log_index: &Option<u64>, last_log_index: &Option<u64>) -> u64 {
    match (matched_log_index, last_log_index) {
        (_, None) => 0,
        (None, Some(last)) => last.saturating_add(1),
        (Some(matched), Some(last)) => last.saturating_sub(*matched),
    }
}

#[cfg(test)]
mod test {
    use crate::core::replication_state::replication_lag;

    #[test]
    fn test_replication_lag() -> anyhow::Result<()> {
        assert_eq!(0, replication_lag(&None, &None));
        assert_eq!(0, replication_lag(&Some(3), &None));
        assert_eq!(4, replication_lag(&None, &Some(3)));
        assert_eq!(1, replication_lag(&Some(2), &Some(3)));
        assert_eq!(0, replication_lag(&Some(3), &Some(3)));
        assert_eq!(0, replication_lag(&Some(4), &Some(3)));
        Ok(())
    }

    /// At the last index, the lag is the distance itself: no successor of an index is computed,
    /// so nothing overflows, whether overflow checks are on or off.
    #[test]
    fn test_replication_lag_at_the_last_index() -> anyhow::Result<()> {
        assert_eq!(1, replication_lag(&Some(u64::MAX - 1), &Some(u64::MAX)));
        assert_eq!(0, replication_lag(&Some(u64::MAX), &Some(u64::MAX)));
        assert_eq!(0, replication_lag(&Some(u64::MAX), &Some(u64::MAX - 1)));
        assert_eq!(u64::MAX, replication_lag(&Some(0), &Some(u64::MAX)));
        assert_eq!(
            u64::MAX,
            replication_lag(&None, &Some(u64::MAX)),
            "u64::MAX + 1 logs saturate"
        );
        Ok(())
    }
}
