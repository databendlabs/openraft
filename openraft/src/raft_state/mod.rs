use std::error::Error;
use std::ops::Deref;

use validit::Validate;

use crate::engine::LogIdList;
use crate::entry::RaftEntry;
use crate::error::ForwardToLeader;
use crate::log_id::RaftLogId;
use crate::node::Node;
use crate::utime::UTime;
use crate::Instant;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;
use crate::ServerState;
use crate::SnapshotMeta;
use crate::Vote;

mod accepted;
pub(crate) mod io_state;
mod log_state_reader;
mod membership_state;
pub(crate) mod snapshot_streaming;
mod vote_state_reader;

pub(crate) use io_state::IOState;
#[allow(unused)] pub(crate) use io_state::LogIOId;

#[cfg(test)]
mod tests {
    mod accepted_test;
    mod forward_to_leader_test;
    mod log_state_reader_test;
    mod read_log_id_test;
    mod validate_test;
}

pub(crate) use accepted::Accepted;
pub(crate) use log_state_reader::LogStateReader;
pub use membership_state::MembershipState;
pub(crate) use vote_state_reader::VoteStateReader;

use crate::display_ext::DisplayOptionExt;
pub(crate) use crate::raft_state::snapshot_streaming::StreamingState;

/// A struct used to represent the raft state which a Raft node needs.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub struct RaftState<NID, N, I>
where
    NID: NodeId,
    N: Node,
    I: Instant,
{
    /// The vote state of this node.
    pub(crate) vote: UTime<Vote<NID>, I>,

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
    /// The state of a Raft node, such as Leader or Follower.
    pub server_state: ServerState,

    pub(crate) accepted: Accepted<NID>,

    pub(crate) io_state: IOState<NID>,

    pub(crate) snapshot_streaming: Option<StreamingState>,

    /// The log id upto which the next time it purges.
    ///
    /// If a log is in use by a replication task, the purge is postponed and is stored in this
    /// field.
    pub(crate) purge_upto: Option<LogId<NID>>,
}

impl<NID, N, I> Default for RaftState<NID, N, I>
where
    NID: NodeId,
    N: Node,
    I: Instant,
{
    fn default() -> Self {
        Self {
            vote: UTime::default(),
            committed: None,
            purged_next: 0,
            log_ids: LogIdList::default(),
            membership_state: MembershipState::default(),
            snapshot_meta: SnapshotMeta::default(),
            server_state: ServerState::default(),
            accepted: Accepted::default(),
            io_state: IOState::default(),
            snapshot_streaming: None,
            purge_upto: None,
        }
    }
}

impl<NID, N, I> LogStateReader<NID> for RaftState<NID, N, I>
where
    NID: NodeId,
    N: Node,
    I: Instant,
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

    fn io_applied(&self) -> Option<&LogId<NID>> {
        self.io_state.applied()
    }

    fn io_snapshot_last_log_id(&self) -> Option<&LogId<NID>> {
        self.io_state.snapshot()
    }

    fn io_purged(&self) -> Option<&LogId<NID>> {
        self.io_state.purged()
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

impl<NID, N, I> VoteStateReader<NID> for RaftState<NID, N, I>
where
    NID: NodeId,
    N: Node,
    I: Instant,
{
    fn vote_ref(&self) -> &Vote<NID> {
        self.vote.deref()
    }
}

impl<NID, N, I> Validate for RaftState<NID, N, I>
where
    NID: NodeId,
    N: Node,
    I: Instant,
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.purged_next == 0 {
            validit::less_equal!(self.log_ids.first().index(), Some(0));
        } else {
            validit::equal!(self.purged_next, self.log_ids.first().next_index());
        }

        validit::less_equal!(self.last_purged_log_id(), self.purge_upto());
        if self.snapshot_last_log_id().is_none() {
            // There is no snapshot, it is possible the application does not store snapshot, and
            // just restarted. it is just ok.
            // In such a case, we assert the monotonic relation without  snapshot-last-log-id
            validit::less_equal!(self.purge_upto(), self.committed());
        } else {
            validit::less_equal!(self.purge_upto(), self.snapshot_last_log_id());
        }
        validit::less_equal!(self.snapshot_last_log_id(), self.committed());
        validit::less_equal!(self.committed(), self.last_log_id());

        self.membership_state.validate()?;

        Ok(())
    }
}

impl<NID, N, I> RaftState<NID, N, I>
where
    NID: NodeId,
    N: Node,
    I: Instant,
{
    /// Get a reference to the current vote.
    pub fn vote_ref(&self) -> &Vote<NID> {
        self.vote.deref()
    }

    /// Return the last updated time of the vote.
    pub fn vote_last_modified(&self) -> Option<I> {
        self.vote.utime()
    }

    /// Get the log id for a linearizable read.
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    pub(crate) fn get_read_log_id(&self) -> Option<&LogId<NID>> {
        // Get the first known log id appended by the last leader.
        // - This log may not be committed.
        // - The leader blank log may have been purged and this could be the last purged log id.
        // - There must be such an entry, which is guaranteed by `Engine::establish_leader()`.
        let leader_first = self.log_ids.by_last_leader().first();

        debug_assert_eq!(
            leader_first.map(|log_id| *log_id.committed_leader_id()),
            self.vote_ref().committed_leader_id(),
            "leader_first must belong to a leader of current vote: leader_first: {}, vote.committed_leader_id: {}",
            leader_first.map(|log_id| log_id.committed_leader_id()).display(),
            self.vote_ref().committed_leader_id().display(),
        );

        let committed = self.committed();

        std::cmp::max(leader_first, committed)
    }

    /// Return the accepted last log id of the current leader.
    pub(crate) fn accepted(&self) -> Option<&LogId<NID>> {
        self.accepted.last_accepted_log_id(self.vote_ref().leader_id())
    }

    /// Update the accepted log id for the current leader.
    pub(crate) fn update_accepted(&mut self, accepted: Option<LogId<NID>>) {
        debug_assert!(
            self.vote_ref().is_committed(),
            "vote must be committed: {}",
            self.vote_ref()
        );
        debug_assert!(
            self.vote_ref().leader_id() >= self.accepted.leader_id(),
            "vote.leader_id: {} must be >= accepted.leader_id: {}",
            self.vote_ref().leader_id(),
            self.accepted.leader_id()
        );

        if accepted.as_ref() > self.accepted.last_accepted_log_id(self.vote_ref().leader_id()) {
            self.accepted = Accepted::new(*self.vote_ref().leader_id(), accepted);
        }
    }

    /// Append a list of `log_id`.
    ///
    /// The log ids in the input has to be continuous.
    pub(crate) fn extend_log_ids_from_same_leader<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_ids: &[LID]) {
        self.log_ids.extend_from_same_leader(new_log_ids)
    }

    pub(crate) fn extend_log_ids<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_id: &[LID]) {
        self.log_ids.extend(new_log_id)
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

    pub(crate) fn io_state_mut(&mut self) -> &mut IOState<NID> {
        &mut self.io_state
    }

    // TODO: move these doc to the [`IOState`]
    /// Returns the state of the already happened IO.
    ///
    /// [`RaftState`] stores the expected state when all queued IO are completed,
    /// which may advance the actual IO state.
    ///
    /// [`IOState`] stores the actual state of the storage.
    ///
    /// Usually, when a client request is handled, [`RaftState`] is updated and several IO command
    /// is enqueued. And when the IO commands are completed, [`IOState`] is updated.
    pub(crate) fn io_state(&self) -> &IOState<NID> {
        &self.io_state
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
    ///
    /// See [Determine Server State][] for more details about determining the server state.
    ///
    /// [Determine Server State]: crate::docs::data::vote#vote-and-membership-define-the-server-state
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn calc_server_state(&self, id: &NID) -> ServerState {
        tracing::debug!(
            contains = display(self.membership_state.contains(id)),
            is_voter = display(self.is_voter(id)),
            is_leader = display(self.is_leader(id)),
            is_leading = display(self.is_leading(id)),
            "states"
        );

        // Openraft does not require Leader/Candidate to be a voter, i.e., a learner node could
        // also be possible to be a leader. Although currently it is not supported.
        // Allowing this will simplify leader step down: The leader just run as long as it wants to,
        #[allow(clippy::collapsible_else_if)]
        if self.is_leader(id) {
            ServerState::Leader
        } else if self.is_leading(id) {
            ServerState::Candidate
        } else {
            if self.is_voter(id) {
                ServerState::Follower
            } else {
                ServerState::Learner
            }
        }
    }

    pub(crate) fn is_voter(&self, id: &NID) -> bool {
        self.membership_state.is_voter(id)
    }

    /// The node is candidate(leadership is not granted by a quorum) or leader(leadership is granted
    /// by a quorum)
    ///
    /// Note that in Openraft Leader does not have to be a voter. See [Determine Server State][] for
    /// more details about determining the server state.
    ///
    /// [Determine Server State]: crate::docs::data::vote#vote-and-membership-define-the-server-state
    pub(crate) fn is_leading(&self, id: &NID) -> bool {
        self.membership_state.contains(id) && self.vote.leader_id().voted_for().as_ref() == Some(id)
    }

    /// The node is leader
    ///
    /// Leadership is granted by a quorum and the vote is committed.
    ///
    /// Note that in Openraft Leader does not have to be a voter. See [Determine Server State][] for
    /// more details about determining the server state.
    ///
    /// [Determine Server State]: crate::docs::data::vote#vote-and-membership-define-the-server-state
    pub(crate) fn is_leader(&self, id: &NID) -> bool {
        self.is_leading(id) && self.vote.is_committed()
    }

    pub(crate) fn assign_log_ids<'a, Ent: RaftEntry<NID, N> + 'a>(
        &mut self,
        entries: impl IntoIterator<Item = &'a mut Ent>,
    ) {
        let mut log_id = LogId::new(
            self.vote_ref().committed_leader_id().unwrap(),
            self.last_log_id().next_index(),
        );
        for entry in entries {
            entry.set_log_id(&log_id);
            tracing::debug!("assign log id: {}", log_id);
            log_id.index += 1;
        }
    }

    /// Build a ForwardToLeader error that contains the leader id and node it knows.
    pub(crate) fn forward_to_leader(&self) -> ForwardToLeader<NID, N> {
        let vote = self.vote_ref();

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
