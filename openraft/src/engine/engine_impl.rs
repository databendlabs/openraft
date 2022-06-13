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
use crate::error::RejectVoteRequest;
use crate::leader::Leader;
use crate::membership::EffectiveMembership;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::RaftState;
use crate::summary::MessageSummary;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::MembershipState;
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
        let quorum_granted = leader.is_granted_by(&self.state.membership_state.effective.membership);

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

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<NID>) -> VoteResponse<NID> {
        tracing::debug!(req = display(req.summary()), "Engine::handle_vote_res");
        tracing::debug!(
            my_vote = display(self.state.vote.summary()),
            my_last_log_id = display(self.state.last_purged_log_id.summary()),
            "Engine::handle_vote_res"
        );

        let res = self.internal_handle_vote_req(&req.vote, &req.last_log_id);
        let vote_granted = if let Err(reject) = res {
            tracing::debug!(
                req = display(req.summary()),
                err = display(reject),
                "reject vote request"
            );
            false
        } else {
            true
        };

        VoteResponse {
            // Return the updated vote, this way the candidate knows which vote is granted, in case the candidate's vote
            // is changed after sending the vote request.
            vote: self.state.vote,
            vote_granted,
            last_log_id: self.state.last_log_id,
        }
    }

    #[tracing::instrument(level = "debug", skip(self, resp))]
    pub(crate) fn handle_vote_resp(&mut self, target: NID, resp: VoteResponse<NID>) {
        tracing::debug!(
            resp = display(resp.summary()),
            target = display(target),
            "handle_vote_resp"
        );
        tracing::debug!(
            my_vote = display(self.state.vote),
            my_last_log_id = display(self.state.last_log_id.summary()),
            "handle_vote_resp"
        );

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
            if self.state.membership_state.effective.all_members().contains(&self.id) {
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

            let quorum_granted = leader.is_granted_by(&self.state.membership_state.effective.membership);
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

    /// Append new log entries by a leader.
    ///
    /// Also Update effective membership if the payload contains
    /// membership config.
    ///
    /// If there is a membership config log entry, the caller has to guarantee the previous one is committed.
    ///
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

        let mut membership = None;
        for entry in entries.iter() {
            if let Some(_m) = entry.get_membership() {
                debug_assert!(membership.is_none());

                let em = Arc::new(EffectiveMembership::from(entry));
                self.state.membership_state.effective = em.clone();
                membership = Some(em);

                self.push_command(Command::UpdateMembership {
                    membership: self.state.membership_state.effective.clone(),
                });
            }
        }

        // If membership does not change, try fast commit.
        self.fast_commit(&membership, entries.last().unwrap());

        // Still need to replicate to learners, even when it is fast-committed.
        self.push_command(Command::ReplicateInputEntries { range: 0..l });
        self.push_command(Command::MoveInputCursorBy { n: l });
    }

    /// Follower/Learner append entries.
    ///
    /// It assumes:
    /// - Previous entries all match.
    /// - conflicting entries are deleted.
    ///
    /// Membership config changes are also detected and applied here.
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn follower_append_entries<'a, Ent: RaftEntry<NID> + 'a>(&mut self, entries: &[Ent]) {
        let l = entries.len();
        if l == 0 {
            return;
        }

        debug_assert_eq!(
            entries[0].get_log_id().index,
            self.state.log_ids.last().cloned().next_index(),
        );

        debug_assert!(entries[0].get_log_id() > self.state.log_ids.last().unwrap());

        self.state.extend_log_ids(entries);
        self.state.last_log_id = entries.last().map(|x| *x.get_log_id());

        self.push_command(Command::AppendInputEntries { range: 0..l });
        self.follower_update_membership(entries.iter());

        self.push_command(Command::MoveInputCursorBy { n: l });
    }

    /// Delete log entries since log index `since`, inclusive, when the log at `since` is found conflict with the
    /// leader.
    ///
    /// And revert effective membership to the last committed if it is from the conflicting logs.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn truncate_logs(&mut self, since: u64) {
        tracing::debug!(since = since, "truncate_logs");

        debug_assert!(since >= self.state.last_purged_log_id.next_index());

        let since_log_id = match self.state.get_log_id(since) {
            None => {
                tracing::debug!("trying to delete absent log at: {}", since);
                return;
            }
            Some(x) => x,
        };

        self.state.log_ids.truncate(since);
        self.state.last_log_id = self.state.get_log_id(since - 1);

        self.push_command(Command::DeleteConflictLog { since: since_log_id });

        // If the effective membership is from a conflicting log,
        // the membership state has to revert to the last committed membership config.
        // See: [Effective-membership](https://datafuselabs.github.io/openraft/effective-membership.html)
        //
        // ```text
        // committed_membership, ... since, ... effective_membership // log
        // ^                                    ^
        // |                                    |
        // |                                    last membership      // before deleting since..
        // last membership                                           // after  deleting since..
        // ```

        let effective = self.state.membership_state.effective.clone();
        if Some(since) <= effective.log_id.index() {
            let was_member = self.is_member();

            let committed = self.state.membership_state.committed.clone();

            tracing::debug!(
                effective = debug(&effective),
                committed = debug(&committed),
                "effective membership is in conflicting logs, revert it to last committed"
            );

            debug_assert!(
                committed.log_id.index() < Some(since),
                "committed membership can not conflict with the leader"
            );

            let mem_state = MembershipState {
                committed: committed.clone(),
                effective: committed,
            };

            self.state.membership_state = mem_state;
            self.push_command(Command::UpdateMembership {
                membership: self.state.membership_state.effective.clone(),
            });

            tracing::debug!(effective = debug(&effective), "Done reverting membership");

            let is_member = self.is_member();

            if was_member != is_member {
                if is_member {
                    self.set_server_state(ServerState::Follower);
                } else {
                    self.set_server_state(ServerState::Learner);
                }
            }
        }
    }

    /// Purge log entries upto `upto`, inclusive.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn purge_log(&mut self, upto: LogId<NID>) {
        let st = &mut self.state;
        let log_id = Some(upto);

        if log_id <= st.last_purged_log_id {
            return;
        }

        st.log_ids.purge(&upto);

        st.last_purged_log_id = log_id;

        if st.last_log_id < log_id {
            st.last_log_id = log_id;
        }

        self.push_command(Command::PurgeLog { upto });
    }

    // --- Draft API ---

    // // --- app API ---
    //
    // /// Write a new log entry.
    // pub(crate) fn write(&mut self) -> Result<Vec<AlgoCmd<NID>>, ForwardToLeader<NID>> {}
    //
    // // --- raft protocol API ---
    //
    // //
    // pub(crate) fn handle_append_entries() {}
    // pub(crate) fn handle_install_snapshot() {}
    //
    // pub(crate) fn handle_append_entries_resp() {}
    // pub(crate) fn handle_install_snapshot_resp() {}
}

/// Supporting util
impl<NID: NodeId> Engine<NID> {
    /// Update effective membership config if encountering a membership config log entry.
    fn try_update_membership<Ent: RaftEntry<NID>>(&mut self, entry: &Ent) {
        if let Some(m) = entry.get_membership() {
            let em = EffectiveMembership::from((entry, m.clone()));
            self.state.membership_state.effective = Arc::new(em);

            self.push_command(Command::UpdateMembership {
                membership: self.state.membership_state.effective.clone(),
            });
        }
    }

    /// Commit at once if a single node constitute a quorum.
    fn fast_commit<Ent: RaftEntry<NID>>(
        &mut self,
        prev_membership: &Option<Arc<EffectiveMembership<NID>>>,
        entry: &Ent,
    ) {
        if let Some(m) = prev_membership {
            if !m.membership.is_majority(&self.single_node_cluster) {
                return;
            }
        }

        if !self.state.membership_state.effective.membership.is_majority(&self.single_node_cluster) {
            return;
        }

        let log_id = entry.get_log_id();
        self.state.committed = Some(*log_id);
        self.push_command(Command::LeaderCommit { upto: *log_id });
    }

    /// Update membership state if membership config entries are found.
    #[allow(dead_code)]
    fn follower_update_membership<'a, Ent: RaftEntry<NID> + 'a>(
        &mut self,
        entries: impl DoubleEndedIterator<Item = &'a Ent>,
    ) {
        let was_member = self.is_member();

        let memberships = Self::last_two_memberships(entries);
        if memberships.is_empty() {
            return;
        }

        tracing::debug!(memberships=?memberships, "applying new membership configs received from leader");

        self.update_membership_state(memberships);
        self.push_command(Command::UpdateMembership {
            membership: self.state.membership_state.effective.clone(),
        });

        let is_member = self.is_member();

        tracing::debug!(
            was_member,
            is_member,
            membership = display(self.state.membership_state.effective.membership.summary()),
            "membership updated"
        );

        if was_member == is_member {
            return;
        }

        if is_member {
            self.set_server_state(ServerState::Follower);
        } else {
            self.set_server_state(ServerState::Learner);
        }
    }

    /// Find the last 2 membership entries in a list of entries.
    ///
    /// A follower/learner reverts the effective membership to the previous one,
    /// when conflicting logs are found.
    ///
    /// See: [Effective-membership](https://datafuselabs.github.io/openraft/effective-membership.html)
    fn last_two_memberships<'a, Ent: RaftEntry<NID> + 'a>(
        entries: impl DoubleEndedIterator<Item = &'a Ent>,
    ) -> Vec<EffectiveMembership<NID>> {
        let mut memberships = vec![];

        // Find the last 2 membership config entries: the committed and the effective.
        for ent in entries.rev() {
            if let Some(m) = ent.get_membership() {
                memberships.insert(0, EffectiveMembership::new(Some(*ent.get_log_id()), m.clone()));
                if memberships.len() == 2 {
                    break;
                }
            }
        }

        memberships
    }

    /// Update membership state with the last 2 membership configs found in new log entries
    ///
    /// Return if new membership config is found
    fn update_membership_state(&mut self, memberships: Vec<EffectiveMembership<NID>>) {
        debug_assert!(self.state.membership_state.effective.log_id < memberships[0].log_id);

        let new_mem_state = if memberships.len() == 1 {
            MembershipState {
                committed: self.state.membership_state.effective.clone(),
                effective: Arc::new(memberships[0].clone()),
            }
        } else {
            // len() == 2
            MembershipState {
                committed: Arc::new(memberships[0].clone()),
                effective: Arc::new(memberships[1].clone()),
            }
        };
        self.state.membership_state = new_mem_state;
    }

    fn set_server_state(&mut self, server_state: ServerState) {
        tracing::debug!(id = display(self.id), ?server_state, "set_server_state");

        // TODO: the caller should be very sure about what server-state to set.
        //       The following condition check is copied from old code,
        //       and should be removed.
        //       And currently just use a panic to indicate the new code in Engine should not reach this point.
        // if server_state == State::Follower && !self.state.membership_state.effective.membership.is_member(&self.id) {
        //     self.state.target_state = State::Learner;
        // } else {
        //     self.state.target_state = server_state;
        // }
        if server_state == ServerState::Follower
            && !self.state.membership_state.effective.membership.is_member(&self.id)
        {
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

    /// Check and grant a vote request.
    /// This is used by all 3 RPC append-entries, vote, install-snapshot to check the `vote` field.
    ///
    /// Grant vote if [vote, last_log_id] >= mine by vector comparison.
    /// Note: A greater `vote` won't be stored if `last_log_id` is smaller.
    /// We do not need to upgrade `vote` for a node that can not become leader.
    ///
    /// If the `vote` is committed, i.e., it is granted by a quorum, then the vote holder, e.g. the leader already has
    /// all of the committed logs, thus in such case, we do not need to check `last_log_id`.
    pub(crate) fn internal_handle_vote_req(
        &mut self,
        vote: &Vote<NID>,
        last_log_id: &Option<LogId<NID>>,
    ) -> Result<(), RejectVoteRequest<NID>> {
        // Partial ord compare:
        // Vote does not has to be total ord.
        // `!(a >= b)` does not imply `a < b`.
        if vote >= &self.state.vote {
            // Ok
        } else {
            return Err(RejectVoteRequest::ByVote(self.state.vote));
        }

        if vote.committed {
            // OK: a quorum has already granted this vote, then I'll grant it too.
        } else {
            // Grant non-committed vote
            if last_log_id >= &self.state.last_log_id {
                // OK
            } else {
                return Err(RejectVoteRequest::ByLastLogId(self.state.last_log_id));
            }
        };

        tracing::debug!(%vote, ?last_log_id, "grant vote request" );

        // Grant the vote

        // There is an active leader or an active candidate.
        // Do not elect for a while.
        self.push_command(Command::InstallElectionTimer {});
        if vote.committed {
            // There is an active leader, reject election for a while.
            self.push_command(Command::RejectElection {});
        }

        if vote > &self.state.vote {
            self.state.vote = *vote;
            self.push_command(Command::SaveVote { vote: *vote });
        }

        // I'm no longer a leader.
        self.state.leader = None;

        #[allow(clippy::collapsible_else_if)]
        if self.state.server_state == ServerState::Follower || self.state.server_state == ServerState::Learner {
            // nothing to do
        } else {
            if self.state.membership_state.effective.all_members().contains(&self.id) {
                self.set_server_state(ServerState::Follower);
            } else {
                self.set_server_state(ServerState::Learner);
            }
        }

        Ok(())
    }

    fn is_member(&self) -> bool {
        self.state.membership_state.effective.membership.contains(&self.id)
    }

    fn push_command(&mut self, cmd: Command<NID>) {
        cmd.update_metrics_flags(&mut self.metrics_flags);
        self.commands.push(cmd)
    }
}
