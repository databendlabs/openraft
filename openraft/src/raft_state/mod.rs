use std::error::Error;
use std::ops::Deref;

use validit::Valid;
use validit::Validate;

use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::ServerState;
use crate::engine::LogIdList;
use crate::error::ForwardToLeader;
use crate::log_id::raft_log_id::RaftLogId;
use crate::storage::SnapshotMeta;
use crate::utime::Leased;

pub(crate) mod io_state;
mod log_state_reader;
mod membership_state;
mod runtime_stats;
mod vote_state_reader;

pub(crate) use io_state::IOState;
#[allow(unused)]
pub(crate) use io_state::io_id::IOId;
pub(crate) use runtime_stats::RuntimeStats;

#[allow(unused)]
pub(crate) use crate::base::histogram::PercentileStats;

#[cfg(test)]
mod tests {
    mod forward_to_leader_test;
    mod is_initialized_test;
    mod log_state_reader_test;
    mod update_committed_test;
    mod validate_test;
}

pub(crate) use log_state_reader::LogStateReader;
pub use membership_state::MembershipState;
pub(crate) use vote_state_reader::VoteStateReader;

use crate::display_ext::DisplayOptionExt;
use crate::entry::RaftEntry;
use crate::entry::raft_entry_ext::RaftEntryExt;
use crate::log_id::ref_log_id::RefLogId;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::raft_state::io_state::io_progress::IOProgress;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// A struct used to represent the raft state which a Raft node needs.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub struct RaftState<C>
where C: RaftTypeConfig
{
    /// The vote state of this node.
    pub(crate) vote: Leased<VoteOf<C>, InstantOf<C>>,

    pub(crate) purged_next: u64,

    /// All log ids this node has.
    pub log_ids: LogIdList<C>,

    /// The latest cluster membership configuration found, in log or in state machine.
    pub membership_state: MembershipState<C>,

    /// The metadata of the last snapshot.
    pub snapshot_meta: SnapshotMeta<C>,

    // --
    // -- volatile fields: they are not persisted.
    // --
    /// The state of a Raft node, such as Leader or Follower.
    pub server_state: ServerState,

    pub(crate) io_state: Valid<IOState<C>>,

    /// The log id up to which the next time it purges.
    ///
    /// If a log is in use by a replication task, the purge is postponed and is stored in this
    /// field.
    pub(crate) purge_upto: Option<LogIdOf<C>>,
}

impl<C> Default for RaftState<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self {
            vote: Leased::default(),
            purged_next: 0,
            log_ids: LogIdList::default(),
            membership_state: MembershipState::default(),
            snapshot_meta: SnapshotMeta::default(),
            server_state: ServerState::default(),
            io_state: Valid::new(IOState::default()),
            purge_upto: None,
        }
    }
}

impl<C> LogStateReader<C> for RaftState<C>
where C: RaftTypeConfig
{
    fn ref_log_id(&self, index: u64) -> Option<RefLogId<'_, C>> {
        self.log_ids.ref_at(index)
    }

    fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.log_ids.last()
    }

    fn committed(&self) -> Option<&LogIdOf<C>> {
        self.apply_progress().accepted()
    }

    fn io_applied(&self) -> Option<&LogIdOf<C>> {
        self.io_state.applied()
    }

    fn io_snapshot_last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.io_state.snapshot()
    }

    fn io_purged(&self) -> Option<&LogIdOf<C>> {
        self.io_state.purged()
    }

    fn snapshot_last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.snapshot_meta.last_log_id.as_ref()
    }

    fn purge_upto(&self) -> Option<&LogIdOf<C>> {
        self.purge_upto.as_ref()
    }

    fn last_purged_log_id(&self) -> Option<&LogIdOf<C>> {
        if self.purged_next == 0 {
            return None;
        }
        self.log_ids.first()
    }
}

impl<C> VoteStateReader<C> for RaftState<C>
where C: RaftTypeConfig
{
    fn vote_ref(&self) -> &VoteOf<C> {
        self.vote.deref()
    }
}

impl<C> Validate for RaftState<C>
where C: RaftTypeConfig
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
        self.io_state.validate()?;

        Ok(())
    }
}

impl<C> RaftState<C>
where C: RaftTypeConfig
{
    /// Get a reference to the current vote.
    pub fn vote_ref(&self) -> &VoteOf<C> {
        self.vote.deref()
    }

    /// Return the last updated time of the vote.
    pub fn vote_last_modified(&self) -> Option<InstantOf<C>> {
        self.vote.last_update()
    }

    /// Get the last committed log ID.
    pub fn committed(&self) -> Option<&LogId<C>> {
        self.apply_progress().accepted()
    }

    pub(crate) fn is_initialized(&self) -> bool {
        // initialize() writes a membership config log entry.
        // If there are logs, it is already initialized.
        if self.last_log_id().is_some() {
            return true;
        }

        // If it received a request-vote from other node, it is already initialized.
        if self.vote_ref() != &VoteOf::<C>::default() {
            return true;
        }

        false
    }

    /// Return the accepted IO request(which are going to be submitted and flushed).
    ///
    /// Such as SaveVote or AppendEntries
    pub(crate) fn accepted_log_io(&self) -> Option<&IOId<C>> {
        self.log_progress().accepted()
    }

    /// Updates the accepted IO, including Vote change or AppendEntries IO.
    ///
    /// Returns the previously accepted value.
    pub(crate) fn accept_log_io(&mut self, accepted: IOId<C>) -> Option<IOId<C>> {
        let curr_accepted = self.log_progress().accepted().cloned();

        tracing::debug!(
            "{}: accept_log: current: {}, new_accepted: {}",
            func_name!(),
            curr_accepted.display(),
            accepted
        );

        if cfg!(debug_assertions) {
            let new_vote = accepted.to_app_vote();
            let current_vote = curr_accepted.clone().map(|io_id| io_id.to_app_vote());
            assert!(
                Some(new_vote.as_ref_vote()) >= current_vote.as_ref().map(|x| x.as_ref_vote()),
                "new accepted.committed_vote {} must be >= current accepted.committed_vote: {}",
                new_vote,
                current_vote.display(),
            );
        }

        if Some(&accepted) > curr_accepted.as_ref() {
            self.log_progress_mut().accept(accepted);
        }

        curr_accepted
    }

    /// Append a list of `log_id`.
    ///
    /// The log ids in the input has to be continuous.
    pub(crate) fn extend_log_ids_from_same_leader<'a, I>(&mut self, new_log_ids: I)
    where
        I: IntoIterator<Item = RefLogId<'a, C>>,
        <I as IntoIterator>::IntoIter: DoubleEndedIterator,
    {
        self.log_ids.extend_from_same_leader(new_log_ids)
    }

    pub(crate) fn extend_log_ids<LID, I>(&mut self, new_log_id: I)
    where
        LID: RaftLogId<C>,
        I: IntoIterator<Item = LID>,
        <I as IntoIterator>::IntoIter: ExactSizeIterator,
    {
        self.log_ids.extend(new_log_id)
    }

    /// This method first updates the cluster committed log ID, then uses it to calculate and
    /// update the local committed log ID.
    pub(crate) fn update_committed(&mut self, cluster_committed: LogIOId<C>) {
        tracing::debug!(
            "{}: leader_committed: {}, my_accepted: {}, my_committed: {}",
            func_name!(),
            cluster_committed,
            self.accepted_log_io().display(),
            self.committed().display()
        );

        self.io_state_mut().cluster_committed.try_update(cluster_committed.clone()).ok();

        let local_committed = self.io_state.calculate_local_committed();

        self.update_local_committed(&local_committed);
    }

    /// Updates the committed log id for local use.
    ///
    /// The local committed log id may be smaller than `cluster_committed` because it requires
    /// all log entries up to this value to be persisted (or guaranteed to be persisted) and
    /// safe from truncation. Only then are these entries safe to apply to the state machine.
    ///
    /// This method updates the committed log id only if the input is greater than the current
    /// value.
    ///
    /// Returns `Some(previous_value)` if updated, or `None` if the input was not greater.
    pub(crate) fn update_local_committed(&mut self, committed: &Option<LogIdOf<C>>) -> Option<Option<LogIdOf<C>>> {
        if committed.as_ref() > self.committed() {
            let prev = self.committed().cloned();

            // Safe unwrap(): committed > self.committed(), implies it cannot be None
            self.apply_progress_mut().accept(committed.clone().unwrap());
            self.membership_state.commit(committed);

            Some(prev)
        } else {
            None
        }
    }

    pub(crate) fn log_progress(&self) -> &IOProgress<IOId<C>> {
        &self.io_state().log_progress
    }

    pub(crate) fn log_progress_mut(&mut self) -> &mut IOProgress<IOId<C>> {
        &mut self.io_state_mut().log_progress
    }

    pub(crate) fn apply_progress(&self) -> &IOProgress<LogId<C>> {
        &self.io_state().apply_progress
    }

    pub(crate) fn apply_progress_mut(&mut self) -> &mut IOProgress<LogId<C>> {
        &mut self.io_state_mut().apply_progress
    }

    pub(crate) fn snapshot_progress_mut(&mut self) -> &mut IOProgress<LogId<C>> {
        &mut self.io_state_mut().snapshot
    }

    pub(crate) fn io_state_mut(&mut self) -> &mut IOState<C> {
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
    pub(crate) fn io_state(&self) -> &IOState<C> {
        &self.io_state
    }

    /// Find the first entry in the input that does not exist on local raft-log,
    /// by comparing the log id.
    pub(crate) fn first_conflicting_index<Ent>(&self, entries: &[Ent]) -> usize
    where Ent: RaftEntry<C> {
        let l = entries.len();

        for (i, ent) in entries.iter().enumerate() {
            let ref_log_id = ent.ref_log_id();

            if !self.has_log_id(ref_log_id) {
                tracing::debug!(
                    at = display(i),
                    entry_log_id = display(ref_log_id),
                    "found nonexistent log id"
                );
                return i;
            }
        }

        tracing::debug!("not found nonexistent");
        l
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_log(&mut self, upto: &LogIdOf<C>) {
        self.purged_next = upto.index() + 1;
        self.log_ids.purge(upto);
    }

    /// Determine the current server state by state.
    ///
    /// See [Determine Server State][] for more details about determining the server state.
    ///
    /// [Determine Server State]: crate::docs::data::vote#vote-and-membership-define-the-server-state
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn calc_server_state(&self, id: &C::NodeId) -> ServerState {
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

    pub(crate) fn is_voter(&self, id: &C::NodeId) -> bool {
        self.membership_state.is_voter(id)
    }

    /// The node is candidate(leadership is not granted by a quorum) or leader(leadership is granted
    /// by a quorum)
    ///
    /// Note that in Openraft Leader does not have to be a voter. See [Determine Server State][] for
    /// more details about determining the server state.
    ///
    /// [Determine Server State]: crate::docs::data::vote#vote-and-membership-define-the-server-state
    pub(crate) fn is_leading(&self, id: &C::NodeId) -> bool {
        self.membership_state.contains(id) && self.vote.leader_node_id() == Some(id)
    }

    /// The node is leader
    ///
    /// Leadership is granted by a quorum and the vote is committed.
    ///
    /// Note that in Openraft Leader does not have to be a voter. See [Determine Server State][] for
    /// more details about determining the server state.
    ///
    /// [Determine Server State]: crate::docs::data::vote#vote-and-membership-define-the-server-state
    pub(crate) fn is_leader(&self, id: &C::NodeId) -> bool {
        self.is_leading(id) && self.vote.is_committed()
    }

    /// Create a Leader using the state of the local `Acceptor`: `Engine.state`.
    ///
    /// This is used when building a Leader without an election,
    /// for example, node-1 elects node-2 as a Leader, node-2 will become a Leader when receives the
    /// vote.
    /// A Leader established with election using the state in `Engine.candidate`.
    pub(crate) fn new_leader(&mut self) -> Leader<C, LeaderQuorumSet<C>> {
        let em = &self.membership_state.effective().membership();

        let last_leader_log_ids = self.log_ids.by_last_leader();

        Leader::new(
            self.vote_ref().to_committed(),
            em.to_quorum_set(),
            em.learner_ids(),
            last_leader_log_ids,
        )
    }

    /// Build a ForwardToLeader error that contains the leader id and node it knows.
    pub(crate) fn forward_to_leader(&self) -> ForwardToLeader<C> {
        let vote = self.vote_ref();

        if vote.is_committed() {
            // Safe unwrap(): vote that is committed has to already have voted for some node.
            let id = vote.to_leader_id().node_id().cloned().unwrap();

            return self.new_forward_to_leader(id);
        }

        ForwardToLeader::empty()
    }

    pub(crate) fn new_forward_to_leader(&self, to: C::NodeId) -> ForwardToLeader<C> {
        // leader may not step down after being removed from `voters`.
        // It does not have to be a voter, being in membership is just enough
        let node = self.membership_state.effective().get_node(&to);

        if let Some(n) = node {
            ForwardToLeader::new(to, n.clone())
        } else {
            tracing::debug!("id={} is not in membership, when getting leader id", to);
            ForwardToLeader::empty()
        }
    }
}
