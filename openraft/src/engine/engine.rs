use std::collections::BTreeSet;
use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
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

        self.push_command(Command::AppendInputEntries { range: 0..l });

        let entry = &mut entries[0];
        if let Some(m) = entry.get_membership() {
            self.check_members_contain_me(m)?;
        } else {
            Err(NotAMembershipEntry {})?;
        }
        self.try_update_membership(entry);

        self.push_command(Command::MoveInputCursorBy { n: l });

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
            self.push_command(Command::SaveVote { vote: self.state.vote });

            // TODO: For compatibility. remove it. The runtime does not need to know about server state.
            self.set_server_state(ServerState::Leader);
            return;
        }

        // Slow-path: send vote request, let a quorum grant it.

        self.push_command(Command::SaveVote { vote: self.state.vote });
        self.push_command(Command::SendVote {
            vote_req: VoteRequest::new(self.state.vote, self.state.last_log_id),
        });

        // TODO: For compatibility. remove it. The runtime does not need to know about server state.
        self.set_server_state(ServerState::Candidate);
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<NID>) -> VoteResponse<NID> {
        let last_log_id = self.state.last_log_id;
        let vote = self.state.vote;

        // Grant vote if [req.vote, req.last_log_id] >= mine by vector comparison.
        // Note: A higher req.vote still won't be stored if req.last_log_id is smaller.
        // We do not need to upgrade `vote` for some node that can not become leader.
        let vote_granted = req.vote >= vote && req.last_log_id >= last_log_id;

        if vote_granted {
            self.push_command(Command::InstallElectionTimer {});

            self.state.vote = req.vote;
            if req.vote > vote {
                self.push_command(Command::SaveVote { vote: req.vote });
            }

            self.state.leader = None;
            #[allow(clippy::collapsible_else_if)]
            if self.state.server_state == ServerState::Follower || self.state.server_state == ServerState::Learner {
                // nothing to do
            } else {
                if self.state.effective_membership.all_members().contains(&self.id) {
                    self.set_server_state(ServerState::Follower);
                } else {
                    self.set_server_state(ServerState::Learner);
                }
            }
        }

        tracing::debug!(?req, %vote, ?last_log_id, %vote_granted, "handle vote request" );

        VoteResponse {
            // Return the updated vote, this way the candidate knows which vote is granted, in case the candidate's vote
            // is changed after sending the vote request.
            vote: self.state.vote,
            vote_granted,
            last_log_id,
        }
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
            self.push_command(Command::SaveVote { vote: self.state.vote });
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
                self.push_command(Command::SaveVote { vote: self.state.vote });

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

        self.push_command(Command::AppendInputEntries { range: 0..l });

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
            self.push_command(Command::Commit { upto: *last_log_id });
        }

        // TODO: only leader need to do this. currently only leader call this method.
        // still need to replicate to learners
        self.push_command(Command::ReplicateInputEntries { range: 0..l });

        self.push_command(Command::MoveInputCursorBy { n: l });
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

            self.push_command(Command::UpdateMembership { membership: m.clone() });
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
        self.push_command(Command::UpdateServerState { server_state });
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

    fn push_command(&mut self, cmd: Command<NID>) {
        cmd.update_metrics_flags(&mut self.metrics_flags);
        self.commands.push(cmd)
    }
}
