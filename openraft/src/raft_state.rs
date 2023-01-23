use std::error::Error;

use crate::engine::LogIdList;
use crate::equal;
use crate::less_equal;
use crate::node::Node;
use crate::raft_types::RaftLogId;
use crate::validate::Validate;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MembershipState;
use crate::NodeId;
use crate::ServerState;
use crate::SnapshotMeta;
use crate::Vote;

/// APIs to get significant log ids reflecting the raft state.
///
/// See: https://datafuselabs.github.io/openraft/log-data-layout
pub(crate) trait LogStateReader<NID: NodeId> {
    /// Get previous log id, i.e., the log id at index - 1
    fn prev_log_id(&self, index: u64) -> Option<LogId<NID>> {
        if index == 0 {
            None
        } else {
            self.get_log_id(index - 1)
        }
    }

    /// Get the log id at the specified index.
    ///
    /// It will return `last_purged_log_id` if index is at the last purged index.
    /// If the log at the specified index is smaller than `last_purged_log_id`, or greater than `last_log_id`, it
    /// returns None.
    fn get_log_id(&self, index: u64) -> Option<LogId<NID>>;

    /// The last known log id in the store.
    ///
    /// The range of all stored log ids are `(last_purged_log_id(), last_log_id()]`, left open right close.
    fn last_log_id(&self) -> Option<&LogId<NID>>;

    /// The last known committed log id, i.e., the id of the log that is accepted by a quorum of voters.
    fn committed(&self) -> Option<&LogId<NID>>;

    /// Return the last log id the snapshot includes.
    fn snapshot_last_log_id(&self) -> Option<&LogId<NID>>;

    /// The greatest log id that has been purged after being applied to state machine, i.e., the oldest known log id.
    ///
    /// The range of log entries that exist in storage is `(last_purged_log_id(), last_log_id()]`,
    /// left open and right close.
    ///
    /// `last_purged_log_id == last_log_id` means there is no log entry in the storage.
    fn last_purged_log_id(&self) -> Option<&LogId<NID>>;
}

/// A struct used to represent the raft state which a Raft node needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaftState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// The vote state of this node.
    pub vote: Vote<NID>,

    /// The LogId of the last log committed(AKA applied) to the state machine.
    ///
    /// - Committed means: a log that is replicated to a quorum of the cluster and it is of the term of the leader.
    ///
    /// - A quorum could be a uniform quorum or joint quorum.
    pub committed: Option<LogId<NID>>,

    pub(crate) purged_next: u64,

    /// All log ids this node has.
    pub log_ids: LogIdList<NID>,

    /// The latest cluster membership configuration found, in log or in state machine.
    pub membership_state: MembershipState<NID, N>,

    /// The metadata of the last snapshot.
    pub snapshot_meta: SnapshotMeta<NID, N>,

    // --
    // -- volatile fields: they are not persisted.
    // --
    pub server_state: ServerState,

    /// The log id upto which the next time it purges.
    ///
    /// If a log is in use by a replication task, the purge is postponed and is stored in this field.
    pub(crate) purge_upto: Option<LogId<NID>>,
}

impl<NID, N> LogStateReader<NID> for RaftState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn get_log_id(&self, index: u64) -> Option<LogId<NID>> {
        self.log_ids.get(index)
    }

    fn last_log_id(&self) -> Option<&LogId<NID>> {
        self.log_ids.last()
    }

    fn committed(&self) -> Option<&LogId<NID>> {
        self.committed.as_ref()
    }

    fn snapshot_last_log_id(&self) -> Option<&LogId<NID>> {
        self.snapshot_meta.last_log_id.as_ref()
    }

    fn last_purged_log_id(&self) -> Option<&LogId<NID>> {
        if self.purged_next == 0 {
            return None;
        }
        self.log_ids.first()
    }
}

impl<NID, N> Validate for RaftState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.purged_next == 0 {
            less_equal!(self.log_ids.first().index(), Some(0));
        } else {
            equal!(self.purged_next, self.log_ids.first().next_index());
        }

        less_equal!(self.last_purged_log_id(), self.purge_upto.as_ref());
        less_equal!(self.purge_upto.as_ref(), self.snapshot_last_log_id());
        less_equal!(self.snapshot_last_log_id(), self.committed());
        less_equal!(self.committed(), self.last_log_id());

        Ok(())
    }
}

impl<NID, N> RaftState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Append a list of `log_id`.
    ///
    /// The log ids in the input has to be continuous.
    pub(crate) fn extend_log_ids_from_same_leader<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_ids: &[LID]) {
        self.log_ids.extend_from_same_leader(new_log_ids)
    }

    #[allow(dead_code)]
    pub(crate) fn extend_log_ids<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_id: &[LID]) {
        self.log_ids.extend(new_log_id)
    }

    /// Return if a log id exists.
    ///
    /// It assumes a committed log will always be chosen, according to raft spec.
    #[allow(dead_code)]
    pub(crate) fn has_log_id(&self, log_id: &LogId<NID>) -> bool {
        if log_id.index < self.committed.next_index() {
            debug_assert!(Some(*log_id) <= self.committed);
            return true;
        }

        // The local log id exists at the index and is same as the input.
        if let Some(local) = self.get_log_id(log_id.index) {
            *log_id == local
        } else {
            false
        }
    }

    /// Return true if the currently effective membership is committed.
    pub(crate) fn is_membership_committed(&self) -> bool {
        self.committed >= self.membership_state.effective.log_id
    }

    /// Update field `committed` if the input is greater.
    /// If updated, it returns the previous value in a `Some()`.
    pub(crate) fn update_committed(&mut self, committed: &Option<LogId<NID>>) -> Option<Option<LogId<NID>>> {
        if committed > &self.committed {
            let prev = self.committed;

            self.committed = *committed;

            // TODO(xp): use a vec to store committed and effective membership.
            if self.committed >= self.membership_state.effective.log_id {
                self.membership_state.committed = self.membership_state.effective.clone();
            }

            Some(prev)
        } else {
            None
        }
    }

    pub(crate) fn purge_log(&mut self, upto: &LogId<NID>) {
        self.purged_next = upto.index + 1;
        self.log_ids.purge(upto);
    }
}
