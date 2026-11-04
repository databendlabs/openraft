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
    /// The follower's local committed log ID, whose prefix is shared with the accepted leader.
    ///
    /// Local writes through this position may still be waiting to flush.
    pub committed_log_id: Option<LogIdOf<C>>,

    /// The follower's in-memory tail after truncating the conflicting suffix.
    pub last_log_id: Option<LogIdOf<C>>,

    /// Selected retained log IDs strictly after `committed_log_id`, normally recent leader ends.
    ///
    /// The list may omit leaders. Each position independently proves a common prefix when its
    /// committed leader ID also occurs in the leader's log.
    pub log_ids: Vec<LogIdOf<C>>,
}

impl<C> ConflictHint<C>
where C: RaftTypeConfig
{
    /// Create a hint from the local committed boundary, tail, and selected later log IDs.
    #[since(version = "0.10.0")]
    pub fn new(
        committed_log_id: Option<LogIdOf<C>>,
        last_log_id: Option<LogIdOf<C>>,
        log_ids: Vec<LogIdOf<C>>,
    ) -> Self {
        Self {
            committed_log_id,
            last_log_id,
            log_ids,
        }
    }

    pub(crate) fn probe_index(&self, local: &LogIdList<CommittedLeaderIdOf<C>>) -> Option<u64> {
        if self.last_log_id.is_none() {
            return Some(0);
        }
        let candidates = self.log_ids.iter().chain(self.last_log_id.iter());
        let mut next = None;
        if let Some(committed) = &self.committed_log_id {
            next = Some(committed.index() + 1);
        }
        for remote in candidates {
            let Some(matching) = local.matching_prefix(remote) else {
                continue;
            };
            next = std::cmp::max(next, Some(matching.index() + 1));
        }
        next
    }

    pub(crate) fn searching_end(&self, local: &LogIdList<CommittedLeaderIdOf<C>>, limit: u64) -> u64 {
        let mut end = std::cmp::min(limit, self.last_log_id.next_index());
        let candidates = self.log_ids.iter().chain(self.last_log_id.iter());
        for remote in candidates {
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
        let committed = Some(log_id(1, 1, 10));
        let hint = ConflictHint::<UTConfig>::new(committed, Some(log_id(2, 1, 80)), vec![log_id(1, 1, 50)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(41), probe);
        assert_eq!(41, end);
    }

    #[test]
    fn matching_tail_guides_probe_after_shorter_follower() {
        let local = LogIdList::new(None, [log_id(1, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(None, Some(log_id(1, 1, 50)), vec![]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(51), probe);
        assert_eq!(51, end);
    }

    #[test]
    fn purged_ids_do_not_prove_a_mismatch_or_a_usable_prefix() {
        let local = LogIdList::new(Some(log_id(1, 1, 40)), [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(None, Some(log_id(2, 1, 80)), vec![log_id(1, 1, 20)]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(None, probe);
        assert_eq!(80, end);
    }

    #[test]
    fn empty_follower_starts_at_zero() {
        let local = LogIdList::new(None, [log_id(1, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(None, None, vec![]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(0), probe);
        assert_eq!(0, end);
    }

    #[test]
    fn different_nodes_in_one_term_are_different_leaders() {
        let local = LogIdList::new(None, [log_id(3, 1, 100)]);
        let hint = ConflictHint::<UTConfig>::new(None, Some(log_id(3, 2, 80)), vec![]);
        let probe = hint.probe_index(&local);
        assert_eq!(None, probe);
    }

    #[test]
    fn committed_anchor_guides_probe_without_matching_suffix_samples() {
        let local = LogIdList::new(None, [log_id(1, 1, 40), log_id(3, 1, 100)]);
        let committed = Some(log_id(1, 1, 30));
        let hint = ConflictHint::<UTConfig>::new(committed, Some(log_id(2, 1, 80)), vec![]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(31), probe);
        assert_eq!(80, end);
    }

    #[test]
    fn purged_committed_anchor_still_bounds_probe() {
        let local = LogIdList::new(Some(log_id(2, 1, 40)), [log_id(4, 1, 100)]);
        let committed = Some(log_id(1, 1, 20));
        let hint = ConflictHint::<UTConfig>::new(committed, Some(log_id(3, 1, 80)), vec![]);
        let probe = hint.probe_index(&local);
        let end = hint.searching_end(&local, 100);
        assert_eq!(Some(21), probe);
        assert_eq!(80, end);
    }
}
