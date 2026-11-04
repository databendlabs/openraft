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

    /// Compare the sampled log with `list`, returning the greatest log ID they are known to share
    /// and the first index at which they are known to diverge.
    ///
    /// The sampled log is the log described by `log_ids`.
    ///
    /// For each sample, the greatest log ID of the same leader at or before the sample in `list`
    /// is shared, because the sampled log holds that leader's run up to the sample. A sample whose
    /// leader has no such log ID in `list`, for example because it is purged, contributes no shared
    /// log ID. A run that the samples omit can share a later log ID. The shared log ID is `None` if
    /// no sample contributes one.
    ///
    /// The sampled log can share log IDs with `list` only below the divergence index. The samples
    /// prove a divergence at:
    ///
    /// - the index after the last sample, where the sampled log has no entry;
    /// - the index after the end of a sampled leader's run in `list`, when that run ends before the
    ///   sample, because the sampled log continues the run;
    /// - a sampled index at which `list` holds a different log ID.
    ///
    /// A purged index or an index past the end of `list` is skipped, so the divergence index can
    /// exceed the end of `list`. A run that the samples omit can hide an earlier divergence. An
    /// empty hint returns `(None, 0)`.
    pub(crate) fn compare(&self, list: &LogIdList<CommittedLeaderIdOf<C>>) -> (Option<LogIdOf<C>>, u64) {
        let mut common = None;

        // The sampled log has no entry after its last sample.
        let last_log_id = self.log_ids.last();
        let mut divergence = last_log_id.next_index();

        for sample in &self.log_ids {
            let leader_id = sample.committed_leader_id();
            let index = sample.index();

            if let Some(shared) = list.get_at_or_before(leader_id, index) {
                // `list` ends this leader's run earlier, while the sampled log continues it.
                if shared.index() < index {
                    divergence = std::cmp::min(divergence, shared.index() + 1);
                }

                common = std::cmp::max(common, Some(shared));
            }

            // `list` holds a different log ID at the sampled index.
            let Some(log_id) = list.get(index) else {
                continue;
            };
            if log_id != *sample {
                divergence = std::cmp::min(divergence, index);
            }
        }

        (common, divergence)
    }
}

#[cfg(test)]
mod conflict_hint_test;
