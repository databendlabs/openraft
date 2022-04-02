use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use maplit::btreeset;

use crate::entry::RaftEntry;
use crate::error::InitializeError;
use crate::error::NotAllowed;
use crate::storage::InitialState;
use crate::EffectiveMembership;
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
    pub(crate) state: InitialState<NID>,

    /// Tracks what kind of metrics changed
    pub(crate) metrics_flags: MetricsChangeFlags,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: Vec<Command<NID>>,
}

/// Commands to send to `RaftRuntime` to execute, to update the application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command<NID: NodeId> {
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

    //
    // --- Draft unimplemented commands:
    //
    // TODO:
    #[allow(dead_code)]
    SaveVote {
        vote: Vote<NID>,
    },
    // TODO:
    #[allow(dead_code)]
    SendVote {
        vote: Vote<NID>,
    },

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
    pub(crate) fn new(id: NID, init_state: &InitialState<NID>) -> Self {
        Self {
            id,
            single_node_cluster: btreeset! {id},
            state: init_state.clone(),
            metrics_flags: MetricsChangeFlags::default(),
            commands: vec![],
        }
    }

    /// Check if a raft node is in a state that allows to initialize.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(term=0, node_id=0)`.
    /// See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    pub(crate) fn check_initialize(&self) -> Result<(), InitializeError<NID>> {
        if self.state.last_log_id.is_none() && self.state.vote == Vote::default() {
            return Ok(());
        }

        tracing::error!(?self.state.last_log_id, ?self.state.vote, "Can not initialize");

        Err(InitializeError::NotAllowed(NotAllowed {
            last_log_id: self.state.last_log_id,
            vote: self.state.vote,
        }))
    }

    fn next_log_id(&mut self) -> LogId<NID> {
        let log_id = LogId::new(self.state.vote.leader_id(), self.state.last_log_id.next_index());
        self.state.last_log_id = Some(log_id);

        log_id
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

        let entry = &mut entries[0];

        self.check_initialize()?;

        let log_id = self.next_log_id();
        entry.set_log_id(&log_id);

        tracing::debug!("append initialization log id: {}", log_id);

        self.commands.push(Command::AppendInputEntries { range: 0..l });
        self.metrics_flags.set_data_changed();

        if let Some(m) = entry.get_membership() {
            let em = EffectiveMembership::new(Some(*entry.get_log_id()), m.clone());
            self.state.effective_membership = Arc::new(em);

            self.commands.push(Command::UpdateMembership { membership: m.clone() });
            self.metrics_flags.set_cluster_changed();
        } else {
            panic!("Initialization log has to be a membership config entry",);
        }

        // TODO: set target state. This should be done by Engine but currently it is not.

        self.commands.push(Command::MoveInputCursorBy { n: l });

        Ok(())
    }

    /// Update the state for a new log entry to append. Update effective membership if the payload contains
    /// membership config.
    ///
    /// TODO(xp): There is no check and this method must be called by a leader.
    /// TODO(xp): metrics flag needs to be dealt with.
    /// TODO(xp): if vote indicates this node is not the leader, refuse append
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn leader_append_entries<Ent: RaftEntry<NID>>(&mut self, entries: &mut [Ent]) {
        let l = entries.len();
        if l == 0 {
            return;
        }

        for entry in entries.iter_mut() {
            let log_id = self.next_log_id();
            entry.set_log_id(&log_id);

            tracing::debug!("append log id: {}", log_id);
        }

        self.commands.push(Command::AppendInputEntries { range: 0..l });
        self.metrics_flags.set_data_changed();

        for entry in entries.iter_mut() {
            // TODO: if previous membership is not committed, reject a new change-membership propose.
            //       unless the new config does not change any members but only learners.
            if let Some(m) = entry.get_membership() {
                let em = EffectiveMembership::new(Some(*entry.get_log_id()), m.clone());
                self.state.effective_membership = Arc::new(em);

                self.commands.push(Command::UpdateMembership { membership: m.clone() });
                self.metrics_flags.set_cluster_changed();
            }
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
    // /// Trigger elect
    // pub(crate) fn elect(&mut self) -> Result<Vec<AlgoCmd<NID>>, Infallible> {
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
    // pub(crate) fn handle_vote_resp() {}
    // pub(crate) fn handle_append_entries_resp() {}
    // pub(crate) fn handle_install_snapshot_resp() {}
}
