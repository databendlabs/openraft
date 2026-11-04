use openraft_macros::since;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::engine::LogIdList;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// Follower log positions that can guide another AppendEntries probe.
///
/// These entries may not be flushed. A hint never acknowledges durable replication; the leader
/// must verify the chosen position with AppendEntries before counting it toward a quorum.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[non_exhaustive]
pub struct ConflictHint<C>
where C: RaftTypeConfig
{
    /// Selected follower log IDs, ordered by index and deduplicated.
    ///
    /// The local committed log ID comes first when known, followed by selected later leader
    /// boundaries. The last element is always the in-memory tail after truncation. An empty list
    /// means the follower has no log history, including no snapshot boundary.
    ///
    /// The first element may be uncommitted. The leader must compare every sample with its own
    /// log before using it as a common prefix. The list may omit leaders.
    pub log_ids: Vec<LogIdOf<C>>,
}

impl<C> ConflictHint<C>
where C: RaftTypeConfig
{
    /// Create a hint from ordered log IDs ending at the follower's tail.
    #[since(version = "0.10.0")]
    pub fn new(log_ids: Vec<LogIdOf<C>>) -> Self {
        Self { log_ids }
    }

    /// Get the greatest log ID shared by the sampled runs and `list`.
    pub(crate) fn last_common_log_id(&self, list: &LogIdList<CommittedLeaderIdOf<C>>) -> Option<LogIdOf<C>> {
        let mut last = None;

        for sample in &self.log_ids {
            let leader_id = sample.committed_leader_id();
            let index = sample.index();

            let Some(log_id) = list.get_at_or_before(leader_id, index) else {
                continue;
            };

            last = std::cmp::max(last, Some(log_id));
        }

        last
    }

    /// Get an exclusive upper bound on indices shared by the sampled log and `list`.
    ///
    /// Omitted runs may hide an earlier difference.
    pub(crate) fn common_prefix_upper_bound(&self, list: &LogIdList<CommittedLeaderIdOf<C>>) -> u64 {
        // The sampled log ends at its last sample.
        let last_log_id = self.log_ids.last();
        let mut end = last_log_id.next_index();

        for sample in &self.log_ids {
            let leader_id = sample.committed_leader_id();
            let index = sample.index();

            // `list` ends this leader's run earlier, so the two logs differ right after that end.
            let represented = list.get_at_or_before(leader_id, index);
            if let Some(represented) = represented
                && represented.index() < index
            {
                end = std::cmp::min(end, represented.index() + 1);
            }

            // `list` holds a different log ID at the sampled index.
            let Some(log_id) = list.get(index) else {
                continue;
            };
            if log_id != *sample {
                end = std::cmp::min(end, index);
            }
        }

        end
    }
}

#[cfg(test)]
mod tests {
    use super::ConflictHint;
    use crate::engine::LogIdList;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    #[test]
    fn shared_leader_with_different_end_indices_guides_probe() {
        let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 10), log_id(1, 1, 50), log_id(2, 1, 80)]);
        let common = hint.last_common_log_id(&local);
        let end = hint.common_prefix_upper_bound(&local);
        let expected = Some(log_id(1, 1, 40));
        assert_eq!(expected, common);
        assert_eq!(41, end);
    }

    #[test]
    fn matching_tail_guides_probe_after_shorter_follower() {
        let local = LogIdList::new(None, [log_id(1, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 50)]);
        let common = hint.last_common_log_id(&local);
        let end = hint.common_prefix_upper_bound(&local);
        let expected = Some(log_id(1, 1, 50));
        assert_eq!(expected, common);
        assert_eq!(51, end);
    }

    #[test]
    fn purged_ids_do_not_prove_a_mismatch_or_a_usable_prefix() {
        let local = LogIdList::new(Some(log_id(1, 1, 40)), [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 20), log_id(2, 1, 80)]);
        let common = hint.last_common_log_id(&local);
        let end = hint.common_prefix_upper_bound(&local);
        assert_eq!(None, common);
        assert_eq!(80, end);
    }

    #[test]
    fn empty_follower_starts_at_zero() {
        let local = LogIdList::new(None, [log_id(1, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![]);
        let common = hint.last_common_log_id(&local);
        let end = hint.common_prefix_upper_bound(&local);
        assert_eq!(None, common);
        assert_eq!(0, end);
    }

    #[test]
    fn different_nodes_in_one_term_are_different_leaders() {
        let local = LogIdList::new(None, [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(3, 2, 80)]);
        let common = hint.last_common_log_id(&local);
        assert_eq!(None, common);
    }

    #[test]
    fn first_matching_sample_guides_probe_without_matching_suffix_samples() {
        let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 30), log_id(2, 1, 80)]);
        let common = hint.last_common_log_id(&local);
        let end = hint.common_prefix_upper_bound(&local);
        let expected = Some(log_id(1, 1, 30));
        assert_eq!(expected, common);
        assert_eq!(80, end);
    }

    #[test]
    fn purged_first_sample_is_not_assumed_matching() {
        let local = LogIdList::new(Some(log_id(2, 1, 40)), [log_id(4, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 20), log_id(3, 1, 80)]);
        let common = hint.last_common_log_id(&local);
        let end = hint.common_prefix_upper_bound(&local);
        assert_eq!(None, common);
        assert_eq!(80, end);
    }

}
