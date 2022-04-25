use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::entry::RaftEntry;
use crate::error::InitializeError;
use crate::error::NotAMembershipEntry;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::leader::Leader;
use crate::membership::EffectiveMembership;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::RaftState;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::NodeId;
use crate::Vote;

/// Raft protocol algorithm.
///
/// It implement the complete raft algorithm except does not actually update any states.
/// But instead, it output commands to let a `RaftRuntime` implementation execute them to actually update the states
/// such as append-log or save-vote by execute .
///
/// This structure only contains necessary information to run raft algorithm,
/// but none of the application specific data.
/// TODO: make the fields private
#[derive(Debug, Clone, Default)]
pub(crate) struct Engine<NID: NodeId> {
    /// TODO:
    #[allow(dead_code)]
    pub(crate) id: NID,

    /// Cache of a cluster of only this node.
    ///
    /// It is used to check an early commit if there is only one node in a cluster.
    pub(crate) single_node_cluster: BTreeSet<NID>,

    /// The state of this raft node.
    pub(crate) state: RaftState<NID>,

    /// Tracks what kind of metrics changed
    pub(crate) metrics_flags: MetricsChangeFlags,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: Vec<Command<NID>>,
}

/// Commands to send to `RaftRuntime` to execute, to update the application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command<NID: NodeId> {
    // Update server state, e.g., Leader, Follower etc.
    // TODO: consider remove this variant. A runtime does not need to know about this. It is only meant for metrics
    //       report.
    UpdateServerState {
        server_state: ServerState,
    },

    // Append a `range` of entries in the input buffer.
    AppendInputEntries {
        range: Range<usize>,
    },

    // Commit entries that are already in the store, upto `upto`, inclusive.
    // And send applied result to the client that proposed the entry.
    Commit {
        upto: LogId<NID>,
    },

    // Replicate a `range` of entries in the input buffer.
    ReplicateInputEntries {
        range: Range<usize>,
    },

    // Membership config changed, need to update replication stream etc.
    UpdateMembership {
        membership: Membership<NID>,
    },

    // Move the cursor pointing to an entry in the input buffer.
    MoveInputCursorBy {
        n: usize,
    },

    // Save vote to storage
    SaveVote {
        vote: Vote<NID>,
    },

    // Send vote to all other members
    SendVote {
        vote_req: VoteRequest<NID>,
    },
    //
    // --- Draft unimplemented commands:
    //

    // TODO:
    #[allow(dead_code)]
    PurgeAppliedLog {
        upto: LogId<NID>,
    },
    // TODO:
    #[allow(dead_code)]
    DeleteConflictLog {
        since: LogId<NID>,
    },

    // TODO:
    #[allow(dead_code)]
    BuildSnapshot {},
}

impl<NID: NodeId> Engine<NID> {
    pub(crate) fn new(id: NID, init_state: &RaftState<NID>) -> Self {
        Self {
            id,
            single_node_cluster: btreeset! {id},
            state: init_state.clone(),
            metrics_flags: MetricsChangeFlags::default(),
            commands: vec![],
        }
    }

    /// Update flags for metrics that need to update, according to the queued commands.
    pub(crate) fn update_metrics_flags(&mut self) {
        let f = &mut self.metrics_flags;

        for cmd in self.commands.iter() {
            match cmd {
                Command::UpdateServerState { .. } => f.set_cluster_changed(),
                Command::AppendInputEntries { .. } => f.set_data_changed(),
                Command::Commit { .. } => f.set_data_changed(),
                Command::ReplicateInputEntries { .. } => {}
                Command::UpdateMembership { .. } => f.set_cluster_changed(),
                Command::MoveInputCursorBy { .. } => {}
                Command::SaveVote { .. } => f.set_data_changed(),
                Command::SendVote { .. } => {}
                Command::PurgeAppliedLog { .. } => f.set_data_changed(),
                Command::DeleteConflictLog { .. } => f.set_data_changed(),
                Command::BuildSnapshot { .. } => f.set_data_changed(),
            }
        }
    }

    /// Initialize a node by appending the first log.
    ///
    /// - The first log has to be membership config log.
    /// - The node has to contain no logs at all and the vote is the minimal value. See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    ///
    /// Appending the very first log is slightly different from appending log by a leader or follower.
    /// This step is not confined by the consensus protocol and has to be dealt with differently.
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn initialize<Ent: RaftEntry<NID>>(&mut self, entries: &mut [Ent]) -> Result<(), InitializeError<NID>> {
        let l = entries.len();
        assert_eq!(1, l);

        self.check_initialize()?;

        self.assign_log_ids(entries.iter_mut());
        self.state.extend_log_ids_from_same_leader(entries);

        self.commands.push(Command::AppendInputEntries { range: 0..l });

        let entry = &mut entries[0];
        if let Some(m) = entry.get_membership() {
            self.check_members_contain_me(m)?;
        } else {
            Err(NotAMembershipEntry {})?;
        }
        self.try_update_membership(entry);

        self.commands.push(Command::MoveInputCursorBy { n: l });

        // With the new config, start to elect to become leader
        self.set_server_state(ServerState::Candidate);

        Ok(())
    }

    /// Start to elect this node as leader
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn elect(&mut self) {
        // init election
        self.state.vote = Vote::new(self.state.vote.term + 1, self.id);
        self.state.leader = Some(Leader::new());

        // Safe unwrap()
        let leader = self.state.leader.as_mut().unwrap();
        leader.grant_vote_by(self.id);
        let quorum_granted = leader.is_granted_by(&self.state.effective_membership.membership);

        // Fast-path: if there is only one node in the cluster.

        if quorum_granted {
            self.state.vote.commit();
            self.commands.push(Command::SaveVote { vote: self.state.vote });

            // TODO: For compatibility. remove it. The runtime does not need to know about server state.
            self.set_server_state(ServerState::Leader);
            return;
        }

        // Slow-path: send vote request, let a quorum grant it.

        self.commands.push(Command::SaveVote { vote: self.state.vote });
        self.commands.push(Command::SendVote {
            vote_req: VoteRequest::new(self.state.vote, self.state.last_log_id),
        });

        // TODO: For compatibility. remove it. The runtime does not need to know about server state.
        self.set_server_state(ServerState::Candidate);
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn handle_vote_resp(&mut self, target: NID, resp: VoteResponse<NID>) {
        // If this node is no longer a leader, just ignore the delayed vote_resp.
        let leader = match &mut self.state.leader {
            None => return,
            Some(l) => l,
        };

        // A response of an old vote request. ignore.
        if resp.vote < self.state.vote {
            return;
        }

        // If peer's vote is greater than current vote, revert to follower state.
        if resp.vote > self.state.vote {
            // TODO(xp): This is a simplified impl: revert to follower as soon as seeing a higher `vote`.
            //           When reverted to follower, it waits for heartbeat for 2 second before starting a new round of
            //           election.
            if self.state.effective_membership.all_members().contains(&self.id) {
                self.set_server_state(ServerState::Follower);
            } else {
                self.set_server_state(ServerState::Learner);
            }

            self.state.vote = resp.vote;
            self.state.leader = None;
            self.commands.push(Command::SaveVote { vote: self.state.vote });
            return;
        }

        if resp.vote_granted {
            leader.grant_vote_by(target);

            let quorum_granted = leader.is_granted_by(&self.state.effective_membership.membership);
            if quorum_granted {
                tracing::debug!("quorum granted vote");

                self.state.vote.commit();
                // Saving the vote that is granted by a quorum, AKA committed vote, is not necessary by original raft.
                // Openraft insists doing this because:
                // - Voting is not in the hot path, thus no performance penalty.
                // - Leadership won't be lost if a leader restarted quick enough.
                self.commands.push(Command::SaveVote { vote: self.state.vote });

                self.set_server_state(ServerState::Leader);
            }
        }

        // Seen a higher log. Keep waiting.
    }

    /// Update the state for a new log entry to append. Update effective membership if the payload contains
    /// membership config.
    ///
    /// TODO(xp): There is no check and this method must be called by a leader.
    /// TODO(xp): metrics flag needs to be dealt with.
    /// TODO(xp): if vote indicates this node is not the leader, refuse append
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn leader_append_entries<'a, Ent: RaftEntry<NID> + 'a>(&mut self, entries: &mut [Ent]) {
        let l = entries.len();
        if l == 0 {
            return;
        }

        self.assign_log_ids(entries.iter_mut());
        self.state.extend_log_ids_from_same_leader(entries);

        self.commands.push(Command::AppendInputEntries { range: 0..l });

        for entry in entries.iter_mut() {
            // TODO: if previous membership is not committed, reject a new change-membership propose.
            //       unless the new config does not change any members but only learners.
            self.try_update_membership(entry);
        }

        if self.state.effective_membership.membership.is_majority(&self.single_node_cluster) {
            // already committed
            let last = entries.last().unwrap();
            let last_log_id = last.get_log_id();
            self.state.committed = Some(*last_log_id);
            // TODO: only leader need to do this. currently only leader call this method.
            self.commands.push(Command::Commit { upto: *last_log_id });
        }

        // TODO: only leader need to do this. currently only leader call this method.
        // still need to replicate to learners
        self.commands.push(Command::ReplicateInputEntries { range: 0..l });

        self.commands.push(Command::MoveInputCursorBy { n: l });
    }

    // --- Draft API ---

    // // --- app API ---
    //
    // /// Write a new log entry.
    // pub(crate) fn write(&mut self) -> Result<Vec<AlgoCmd<NID>>, ForwardToLeader<NID>> {
    //     todo!()
    // }
    //
    // // --- raft protocol API ---
    //
    // //
    // pub(crate) fn handle_vote() {}
    // pub(crate) fn handle_append_entries() {}
    // pub(crate) fn handle_install_snapshot() {}
    //
    // pub(crate) fn handle_append_entries_resp() {}
    // pub(crate) fn handle_install_snapshot_resp() {}

    /// Update effective membership config if encountering a membership config log entry.
    fn try_update_membership<Ent: RaftEntry<NID>>(&mut self, entry: &Ent) {
        if let Some(m) = entry.get_membership() {
            let em = EffectiveMembership::new(Some(*entry.get_log_id()), m.clone());
            self.state.effective_membership = Arc::new(em);

            self.commands.push(Command::UpdateMembership { membership: m.clone() });
        }
    }

    fn set_server_state(&mut self, server_state: ServerState) {
        tracing::debug!(id = display(self.id), ?server_state, "set_server_state");

        // TODO: the caller should be very sure about what server-state to set.
        //       The following condition check is copied from old code,
        //       and should be removed.
        //       And currently just use a panic to indicate the new code in Engine should not reach this point.
        // if server_state == State::Follower && !self.state.effective_membership.membership.is_member(&self.id) {
        //     self.state.target_state = State::Learner;
        // } else {
        //     self.state.target_state = server_state;
        // }
        if server_state == ServerState::Follower && !self.state.effective_membership.membership.is_member(&self.id) {
            unreachable!("caller does not know what to do?")
        }

        self.state.server_state = server_state;
        self.commands.push(Command::UpdateServerState { server_state });
    }

    /// Check if a raft node is in a state that allows to initialize.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(term=0, node_id=0)`.
    /// See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    fn check_initialize(&self) -> Result<(), NotAllowed<NID>> {
        if self.state.last_log_id.is_none() && self.state.vote == Vote::default() {
            return Ok(());
        }

        tracing::error!(?self.state.last_log_id, ?self.state.vote, "Can not initialize");

        Err(NotAllowed {
            last_log_id: self.state.last_log_id,
            vote: self.state.vote,
        })
    }

    /// When initialize, the node that accept initialize request has to be a member of the initial config.
    fn check_members_contain_me(&self, m: &Membership<NID>) -> Result<(), NotInMembers<NID>> {
        if !m.is_member(&self.id) {
            let e = NotInMembers {
                node_id: self.id,
                membership: m.clone(),
            };
            Err(e)
        } else {
            Ok(())
        }
    }

    fn assign_log_ids<'a, Ent: RaftEntry<NID> + 'a>(&mut self, entries: impl Iterator<Item = &'a mut Ent>) {
        for entry in entries {
            let log_id = self.next_log_id();
            entry.set_log_id(&log_id);
            tracing::debug!("assign log id: {}", log_id);
        }
    }

    fn next_log_id(&mut self) -> LogId<NID> {
        let log_id = LogId::new(self.state.vote.leader_id(), self.state.last_log_id.next_index());
        self.state.last_log_id = Some(log_id);

        log_id
    }
}
