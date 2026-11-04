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

    pub(crate) fn probe_index(&self, local: &LogIdList<CommittedLeaderIdOf<C>>) -> Option<u64> {
        if self.log_ids.is_empty() {
            return Some(0);
        }
        let mut next = None;
        for remote in &self.log_ids {
            let Some(matching) = local.matching_prefix(remote) else {
                continue;
            };
            next = std::cmp::max(next, Some(matching.index() + 1));
        }
        next
    }

    pub(crate) fn searching_end(&self, local: &LogIdList<CommittedLeaderIdOf<C>>, limit: u64) -> u64 {
        let last_log_id = self.log_ids.last();
        let mut end = std::cmp::min(limit, last_log_id.next_index());
        for remote in &self.log_ids {
            if let Some(prefix) = local.matching_prefix(remote)
                && prefix.index() < remote.index()
            {
                end = std::cmp::min(end, prefix.index() + 1);
            }
            let Some(own) = local.get(remote.index()) else {
                continue;
            };
            if own != *remote {
                end = std::cmp::min(end, remote.index());
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
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(41), probe);
        assert_eq!(41, end);
    }

    #[test]
    fn matching_tail_guides_probe_after_shorter_follower() {
        let local = LogIdList::new(None, [log_id(1, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 50)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(51), probe);
        assert_eq!(51, end);
    }

    #[test]
    fn purged_ids_do_not_prove_a_mismatch_or_a_usable_prefix() {
        let local = LogIdList::new(Some(log_id(1, 1, 40)), [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 20), log_id(2, 1, 80)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(None, probe);
        assert_eq!(80, end);
    }

    #[test]
    fn empty_follower_starts_at_zero() {
        let local = LogIdList::new(None, [log_id(1, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(0), probe);
        assert_eq!(0, end);
    }

    #[test]
    fn different_nodes_in_one_term_are_different_leaders() {
        let local = LogIdList::new(None, [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(3, 2, 80)]);
        let probe = hint.probe_index(&local);
        assert_eq!(None, probe);
    }

    #[test]
    fn first_matching_sample_guides_probe_without_matching_suffix_samples() {
        let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 30), log_id(2, 1, 80)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(31), probe);
        assert_eq!(80, end);
    }

    #[test]
    fn purged_first_sample_is_not_assumed_matching() {
        let local = LogIdList::new(Some(log_id(2, 1, 40)), [log_id(4, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 20), log_id(3, 1, 80)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(None, probe);
        assert_eq!(80, end);
    }

    #[test]
    fn shared_snapshot_boundary_allows_log_replication() {
        let local = LogIdList::new(Some(log_id(1, 1, 40)), [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 50)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(41), probe);
        assert_eq!(41, end);
    }

    #[test]
    fn an_old_sample_does_not_exclude_an_omitted_shared_leader() {
        let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 80), log_id(5, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(vec![log_id(1, 1, 30), log_id(4, 1, 90)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(31), probe);
        assert_eq!(90, end);
    }
}
