use std::error::Error;

use crate::engine::LogIdList;
use crate::entry::RaftEntry;
use crate::equal;
use crate::error::ForwardToLeader;
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

    /// Return if a log id exists.
    ///
    /// It assumes a committed log will always get positive return value, according to raft spec.
    fn has_log_id(&self, log_id: &LogId<NID>) -> bool {
        if log_id.index < self.committed().next_index() {
            debug_assert!(Some(log_id) <= self.committed());
            return true;
        }

        // The local log id exists at the index and is same as the input.
        if let Some(local) = self.get_log_id(log_id.index) {
            *log_id == local
        } else {
            false
        }
    }

    /// Get the log id at the specified index.
    ///
    /// It will return `last_purged_log_id` if index is at the last purged index.
    /// If the log at the specified index is smaller than `last_purged_log_id`, or greater than
    /// `last_log_id`, it returns None.
    fn get_log_id(&self, index: u64) -> Option<LogId<NID>>;

    /// The last known log id in the store.
    ///
    /// The range of all stored log ids are `(last_purged_log_id(), last_log_id()]`, left open right
    /// close.
    fn last_log_id(&self) -> Option<&LogId<NID>>;

    /// The last known committed log id, i.e., the id of the log that is accepted by a quorum of
    /// voters.
    fn committed(&self) -> Option<&LogId<NID>>;

    /// Return the last log id the snapshot includes.
    fn snapshot_last_log_id(&self) -> Option<&LogId<NID>>;

    /// Return the log id it wants to purge up to.
    ///
    /// Logs may not be able to be purged at once because they are in use by replication tasks.
    fn purge_upto(&self) -> Option<&LogId<NID>>;

    /// The greatest log id that has been purged after being applied to state machine, i.e., the
    /// oldest known log id.
    ///
    /// The range of log entries that exist in storage is `(last_purged_log_id(), last_log_id()]`,
    /// left open and right close.
    ///
    /// `last_purged_log_id == last_log_id` means there is no log entry in the storage.
    fn last_purged_log_id(&self) -> Option<&LogId<NID>>;
}

/// APIs to get vote.
pub(crate) trait VoteStateReader<NID: NodeId> {
    /// Get the current vote.
    fn get_vote(&self) -> &Vote<NID>;
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
    /// - Committed means: a log that is replicated to a quorum of the cluster and it is of the term
    ///   of the leader.
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
    /// If a log is in use by a replication task, the purge is postponed and is stored in this
    /// field.
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

    fn purge_upto(&self) -> Option<&LogId<NID>> {
        self.purge_upto.as_ref()
    }

    fn last_purged_log_id(&self) -> Option<&LogId<NID>> {
        if self.purged_next == 0 {
            return None;
        }
        self.log_ids.first()
    }
}

impl<NID, N> VoteStateReader<NID> for RaftState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn get_vote(&self) -> &Vote<NID> {
        &self.vote
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

        less_equal!(self.last_purged_log_id(), self.purge_upto());
        less_equal!(self.purge_upto(), self.snapshot_last_log_id());
        less_equal!(self.snapshot_last_log_id(), self.committed());
        less_equal!(self.committed(), self.last_log_id());

        self.membership_state.validate()?;

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

    pub(crate) fn extend_log_ids<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_id: &[LID]) {
        self.log_ids.extend(new_log_id)
    }

    /// Return true if the currently effective membership is committed.
    pub(crate) fn is_membership_committed(&self) -> bool {
        self.committed() >= self.membership_state.effective().log_id.as_ref()
    }

    /// Update field `committed` if the input is greater.
    /// If updated, it returns the previous value in a `Some()`.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_committed(&mut self, committed: &Option<LogId<NID>>) -> Option<Option<LogId<NID>>> {
        if committed.as_ref() > self.committed() {
            let prev = self.committed().copied();

            self.committed = *committed;
            self.membership_state.commit(committed);

            Some(prev)
        } else {
            None
        }
    }

    /// Find the first entry in the input that does not exist on local raft-log,
    /// by comparing the log id.
    pub(crate) fn first_conflicting_index<Ent>(&self, entries: &[Ent]) -> usize
    where Ent: RaftLogId<NID> {
        let l = entries.len();

        for (i, ent) in entries.iter().enumerate() {
            let log_id = ent.get_log_id();

            if !self.has_log_id(log_id) {
                tracing::debug!(
                    at = display(i),
                    entry_log_id = display(log_id),
                    "found nonexistent log id"
                );
                return i;
            }
        }

        tracing::debug!("not found nonexistent");
        l
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_log(&mut self, upto: &LogId<NID>) {
        self.purged_next = upto.index + 1;
        self.log_ids.purge(upto);
    }

    /// Determine the current server state by state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn calc_server_state(&self, id: &NID) -> ServerState {
        tracing::debug!(
            is_member = display(self.is_voter(id)),
            is_leader = display(self.is_leader(id)),
            is_leading = display(self.is_leading(id)),
            "states"
        );
        if self.is_voter(id) {
            if self.is_leader(id) {
                ServerState::Leader
            } else if self.is_leading(id) {
                ServerState::Candidate
            } else {
                ServerState::Follower
            }
        } else {
            ServerState::Learner
        }
    }

    pub(crate) fn is_voter(&self, id: &NID) -> bool {
        self.membership_state.is_voter(id)
    }

    /// The node is candidate(leadership is not granted by a quorum) or leader(leadership is granted
    /// by a quorum)
    pub(crate) fn is_leading(&self, id: &NID) -> bool {
        self.vote.leader_id().voted_for().as_ref() == Some(id)
    }

    pub(crate) fn is_leader(&self, id: &NID) -> bool {
        self.vote.leader_id().voted_for().as_ref() == Some(id) && self.vote.is_committed()
    }

    pub(crate) fn assign_log_ids<'a, Ent: RaftEntry<NID, N> + 'a>(
        &mut self,
        entries: impl Iterator<Item = &'a mut Ent>,
    ) {
        let mut log_id = LogId::new(
            self.get_vote().committed_leader_id().unwrap(),
            self.last_log_id().next_index(),
        );
        for entry in entries {
            entry.set_log_id(&log_id);
            tracing::debug!("assign log id: {}", log_id);
            log_id.index += 1;
        }
    }

    /// Build a ForwardToLeader error that contains the leader id and node it knows.
    // TODO: This will be used in next PR. delete this attr
    #[allow(dead_code)]
    pub(crate) fn forward_to_leader(&self) -> ForwardToLeader<NID, N> {
        let vote = self.get_vote();

        if vote.is_committed() {
            // Safe unwrap(): vote that is committed has to already have voted for some node.
            let id = vote.leader_id().voted_for().unwrap();

            // leader may not step down after being removed from `voters`.
            // It does not have to be a voter, being in membership is just enough
            let node = self.membership_state.effective().get_node(&id);
            if let Some(n) = node {
                return ForwardToLeader::new(id, n.clone());
            } else {
                tracing::debug!("id={} is not in membership, when getting leader id", id);
            }
        };

        ForwardToLeader::empty()
    }
}
