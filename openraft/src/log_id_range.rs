use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use display_more::DisplayOptionExt;
use validit::Validate;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// A log id range of continuous series of log entries.
///
/// The range of log to send is left open right close: `(prev, last]`.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogIdRange<C>
where C: RaftTypeConfig
{
    /// The prev log id before the first to send, exclusive.
    pub(crate) prev: Option<LogIdOf<C>>,

    /// The last log id to send, inclusive.
    pub(crate) last: Option<LogIdOf<C>>,
}

impl<C> Copy for LogIdRange<C>
where
    C: RaftTypeConfig,
    LogIdOf<C>: Copy,
{
}

impl<C> Display for LogIdRange<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}]", self.prev.display(), self.last.display())
    }
}

impl<C> Validate for LogIdRange<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(&self.prev, &self.last);

        // Log ids compare the leader before the index, so `prev <= last` also holds for a later
        // leader's log at a lower or equal index. The two ends of a nonempty range in one log
        // history have increasing indices. Compare them directly: `u64::MAX` has no successor.
        if self.prev != self.last {
            let prev_index = self.prev.index();
            let last_index = self.last.index();
            validit::less!(prev_index, last_index);
        }
        Ok(())
    }
}

impl<C> LogIdRange<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(prev: Option<LogIdOf<C>>, last: Option<LogIdOf<C>>) -> Self {
        Self { prev, last }
    }
}

#[cfg(test)]
mod tests {
    use openraft_rt_tokio::TokioRuntime;
    use validit::Validate;

    use crate::RaftTypeConfig;
    use crate::engine::testing::UTConfig;
    use crate::log_id_range::LogIdRange;
    use crate::type_config::alias::LeaderIdOf;
    use crate::type_config::alias::LogIdOf;
    use crate::vote::RaftLeaderIdExt;

    crate::declare_raft_types!(
        StdConfig:
            LeaderId = crate::impls::leader_id_std::LeaderId<u64, u64>,
            AsyncRuntime = TokioRuntime,
    );

    #[test]
    fn test_log_id_range_validate_with_advanced_leader_id() {
        assert_validate_results::<UTConfig>();
    }

    #[test]
    fn test_log_id_range_validate_with_standard_leader_id() {
        assert_validate_results::<StdConfig>();
    }

    /// Asserts which ranges `validate()` accepts, with the committed leader ID that `C` puts in a
    /// log ID.
    fn assert_validate_results<C>()
    where C: RaftTypeConfig<Term = u64, NodeId = u64> {
        let log_id = |term: u64, index: u64| Some(LogIdOf::<C>::new(LeaderIdOf::<C>::new_committed(term, 1), index));
        let max = u64::MAX;

        // Each case is `(prev, last, valid)`.
        let cases = [
            // Empty ranges.
            (None, None, true),
            (log_id(1, 5), log_id(1, 5), true),
            (log_id(1, max), log_id(1, max), true),
            // A range that starts at index 0, one in a leader's logs, and one across leaders.
            (None, log_id(1, 0), true),
            (log_id(1, 5), log_id(1, 8), true),
            (log_id(1, 5), log_id(2, 8), true),
            // Ranges that end at index u64::MAX, which has no successor.
            (None, log_id(1, max), true),
            (log_id(1, max - 1), log_id(1, max), true),
            (log_id(1, max - 1), log_id(2, max), true),
            // The last log ID is below prev.
            (log_id(1, 5), None, false),
            (log_id(1, 5), log_id(1, 4), false),
            (log_id(2, 5), log_id(1, 6), false),
            // The last log ID is above prev, from a later leader, but its index is not.
            (log_id(1, 5), log_id(2, 4), false),
            (log_id(1, 5), log_id(2, 5), false),
        ];

        for (prev, last, valid) in cases {
            let range = LogIdRange::<C>::new(prev, last);
            tracing::info!("validate {}, expect valid: {}", range, valid);

            let result = range.validate();
            assert_eq!(valid, result.is_ok(), "{range}: {result:?}");
        }
    }
}
