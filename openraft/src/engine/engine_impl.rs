use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::ServerState;
use crate::engine::Command;
use crate::entry::RaftEntry;
use crate::error::InitializeError;
use crate::error::NotAMembershipEntry;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::error::RejectVoteRequest;
use crate::membership::EffectiveMembership;
use crate::membership::NodeRole;
use crate::progress::Progress;
use crate::raft::AppendEntriesResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::RaftState;
use crate::raft_types::RaftLogId;
use crate::summary::MessageSummary;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::NodeId;
use crate::Vote;

/// Config for Engine
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct EngineConfig {
    /// The maximum number of applied logs to keep before purging.
    pub(crate) max_applied_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    pub(crate) purge_batch_size: u64,

    /// whether to keep applied log that are not included in snapshots.
    /// false by default
    pub(crate) keep_unsnapshoted_log: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_applied_log_to_keep: 1000,
            purge_batch_size: 256,
            keep_unsnapshoted_log: false,
        }
    }
}

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
#[derive(PartialEq, Eq)]
pub(crate) struct Engine<NID: NodeId> {
    /// TODO:
    #[allow(dead_code)]
    pub(crate) id: NID,

    pub(crate) config: EngineConfig,

    /// Last log included in snapshot
    pub(crate) snapshot_last_log_id: Option<LogId<NID>>,

    /// The state of this raft node.
    pub(crate) state: RaftState<NID>,

    /// Tracks what kind of metrics changed
    pub(crate) metrics_flags: MetricsChangeFlags,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: Vec<Command<NID>>,
}

impl<NID: NodeId> Engine<NID> {
    pub(crate) fn new(id: NID, init_state: &RaftState<NID>, config: EngineConfig) -> Self {
        Self {
            id,
            config,
            snapshot_last_log_id: None,
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
        debug_assert_eq!(1, l);

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
        self.enter_leader();
        // TODO: this should be part of the job of enter_leader()
        self.state.vote = Vote::new(self.state.vote.term + 1, self.id);

        // Safe unwrap()
        let leader = self.state.leader.as_mut().unwrap();
        leader.grant_vote_by(self.id);
        let quorum_granted = leader.is_vote_granted();

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
            vote_req: VoteRequest::new(self.state.vote, self.state.last_log_id()),
        });

        // TODO: For compatibility. remove it. The runtime does not need to know about server state.
        self.set_server_state(ServerState::Candidate);
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<NID>) -> VoteResponse<NID> {
        tracing::debug!(req = display(req.summary()), "Engine::handle_vote_req");
        tracing::debug!(
            my_vote = display(self.state.vote.summary()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            "Engine::handle_vote_req"
        );

        let res = if req.last_log_id >= self.state.last_log_id() {
            self.internal_handle_vote_change(&req.vote)
        } else {
            Err(RejectVoteRequest::ByLastLogId(self.state.last_log_id()))
        };

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
            last_log_id: self.state.last_log_id(),
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
            my_last_log_id = display(self.state.last_log_id().summary()),
            "handle_vote_resp"
        );

        // If this node is no longer a leader(i.e., electing), just ignore the delayed vote_resp.
        let leader = match &mut self.state.leader {
            None => return,
            Some(l) => l,
        };

        if resp.vote < self.state.vote {
            debug_assert!(!resp.vote_granted);
        }

        if resp.vote_granted {
            leader.grant_vote_by(target);

            let quorum_granted = leader.is_vote_granted();
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
            return;
        }

        // vote is rejected:

        let node_role = self.state.membership_state.effective.get_node_role(&self.id);
        debug_assert_eq!(Some(NodeRole::Voter), node_role);

        // TODO(xp): This is a simplified impl: revert to follower as soon as seeing a higher `vote`.
        //           When reverted to follower, it waits for heartbeat for 2 second before starting a new round of
        //           election.
        self.set_server_state(ServerState::Follower);
        self.leave_leader();

        // If peer's vote is greater than current vote, revert to follower state.
        if resp.vote > self.state.vote {
            self.state.vote = resp.vote;
            self.push_command(Command::SaveVote { vote: self.state.vote });
        }

        // Seen a higher log.
        if resp.last_log_id > self.state.last_log_id() {
            // TODO: if my log is greater, install a smaller election timeout.
            self.push_command(Command::InstallElectionTimer { can_be_leader: false });
        } else {
            self.push_command(Command::InstallElectionTimer { can_be_leader: true });
        }
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

        // Fast commit:
        // If the cluster has only one voter, then an entry will be committed as soon as it is appended.
        // But if there is a membership log in the middle of the input entries, the condition to commit will change.
        // Thus we have to deal with entries before and after a membership entry differently:
        //
        // When a membership entry is seen, update progress for all former entries.
        // Then upgrade the quorum set for the Progress.
        //
        // E.g., if the input entries are `2..6`, entry 4 changes membership from `a` to `abc`.
        // Then it will output a LeaderCommit command to commit entries `2,3`.
        // ```text
        // 1 2 3 4 5 6
        // a x x a y y
        //       b
        //       c
        // ```
        //
        // If the input entries are `2..6`, entry 4 changes membership from `abc` to `a`.
        // Then it will output a LeaderCommit command to commit entries `2,3,4,5,6`.
        // ```text
        // 1 2 3 4 5 6
        // a x x a y y
        // b
        // c
        // ```
        for entry in entries.iter() {
            if let Some(_m) = entry.get_membership() {
                let log_index = entry.get_log_id().index;

                if log_index > 0 {
                    if let Some(prev_log_id) = self.state.get_log_id(log_index - 1) {
                        self.update_progress(self.id, Some(prev_log_id));
                    }
                }

                // since this entry, the condition to commit has been changed.
                self.update_effective_membership(entry.get_log_id(), _m);
            }
        }
        if let Some(last) = entries.last() {
            self.update_progress(self.id, Some(*last.get_log_id()));
        }

        // Still need to replicate to learners, even when it is fast-committed.
        self.push_command(Command::ReplicateInputEntries { range: 0..l });
        self.push_command(Command::MoveInputCursorBy { n: l });
    }

    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_append_entries_req<'a, Ent>(
        &mut self,
        vote: &Vote<NID>,
        prev_log_id: Option<LogId<NID>>,
        entries: &[Ent],
        leader_committed: Option<LogId<NID>>,
    ) -> AppendEntriesResponse<NID>
    where
        Ent: RaftEntry<NID> + MessageSummary<Ent> + 'a,
    {
        tracing::debug!(
            vote = display(vote),
            prev_log_id = display(prev_log_id.summary()),
            entries = display(entries.summary()),
            leader_committed = display(leader_committed.summary()),
            "append-entries request"
        );
        tracing::debug!(
            my_vote = display(self.state.vote),
            my_last_log_id = display(self.state.last_log_id().summary()),
            my_last_applied = display(self.state.last_applied.summary()),
            "local state"
        );

        let res = self.internal_handle_vote_change(vote);
        if let Err(rejected) = res {
            return rejected.into();
        }

        // Vote is legal. Check if prev_log_id matches local raft-log.

        if let Some(ref prev) = prev_log_id {
            if !self.state.has_log_id(prev) {
                let local = self.state.get_log_id(prev.index);
                tracing::debug!(local = debug(&local), "prev_log_id does not match");

                self.truncate_logs(prev.index);
                return AppendEntriesResponse::Conflict;
            }
        }
        // else `prev_log_id.is_none()` means replicating logs from the very beginning.

        tracing::debug!(
            ?self.state.committed,
            entries = %entries.summary(),
            "prev_log_id matches, skip matching entries",
        );

        let l = entries.len();
        let since = self.first_conflicting_index(entries);
        if since < l {
            // Before appending, if an entry overrides an conflicting one,
            // the entries after it has to be deleted first.
            // Raft requires log ids are in total order by (term,index).
            // Otherwise the log id with max index makes committed entry invisible in election.
            self.truncate_logs(entries[since].get_log_id().index);
            self.follower_do_append_entries(entries, since);
        }

        self.follower_commit_entries(leader_committed, prev_log_id, entries);

        AppendEntriesResponse::Success
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn follower_commit_entries<'a, Ent: RaftEntry<NID> + 'a>(
        &mut self,
        leader_committed: Option<LogId<NID>>,
        prev_log_id: Option<LogId<NID>>,
        entries: &[Ent],
    ) {
        tracing::debug!(
            leader_committed = display(leader_committed.summary()),
            prev_log_id = display(prev_log_id.summary()),
        );

        // Committed index can not > last_log_id.index
        let last = entries.last().map(|x| *x.get_log_id());
        let last = std::cmp::max(last, prev_log_id);
        let committed = std::cmp::min(leader_committed, last);

        tracing::debug!(committed = display(committed.summary()), "update committed");

        if let Some(prev_committed) = self.state.update_committed(&committed) {
            self.push_command(Command::FollowerCommit {
                // TODO(xp): when restart, commit is reset to None. Use last_applied instead.
                since: prev_committed,
                upto: committed.unwrap(),
            });
            self.purge_applied_log();
        }
    }

    /// Follower/Learner appends `entries[since..]`.
    ///
    /// It assumes:
    /// - Previous entries all match.
    /// - conflicting entries are deleted.
    ///
    /// Membership config changes are also detected and applied here.
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn follower_do_append_entries<'a, Ent: RaftEntry<NID> + 'a>(&mut self, entries: &[Ent], since: usize) {
        let l = entries.len();
        if since == l {
            return;
        }

        let entries = &entries[since..];

        debug_assert_eq!(
            entries[0].get_log_id().index,
            self.state.log_ids.last().cloned().next_index(),
        );

        debug_assert!(Some(entries[0].get_log_id()) > self.state.log_ids.last());

        self.state.extend_log_ids(entries);

        self.push_command(Command::AppendInputEntries { range: since..l });
        self.follower_update_membership(entries.iter());

        // TODO(xp): should be moved to handle_append_entries_req()
        self.push_command(Command::MoveInputCursorBy { n: l });
    }

    /// Delete log entries since log index `since`, inclusive, when the log at `since` is found conflict with the
    /// leader.
    ///
    /// And revert effective membership to the last committed if it is from the conflicting logs.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn truncate_logs(&mut self, since: u64) {
        tracing::debug!(since = since, "truncate_logs");

        debug_assert!(since >= self.state.last_purged_log_id().next_index());

        let since_log_id = match self.state.get_log_id(since) {
            None => {
                tracing::debug!("trying to delete absent log at: {}", since);
                return;
            }
            Some(x) => x,
        };

        self.state.log_ids.truncate(since);

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
            let server_state = self.calc_server_state();

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

            self.update_server_state_if_changed(server_state);
        }
    }

    /// Purge applied log if needed.
    ///
    /// `max_applied_log_to_keep` specifies the number of applied logs to keep.
    /// `max_applied_log_to_keep==0` means every applied log can be purged.
    // NOTE: simple method, not tested.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_applied_log(&mut self) {
        if let Some(purge_upto) = self.calc_purge_upto() {
            self.purge_log(purge_upto);
        }
    }

    /// Calculate the log id up to which to purge, inclusive.
    ///
    /// Only applied log will be purged.
    /// It may return None if there is no log to purge.
    ///
    /// `max_keep` specifies the number of applied logs to keep.
    /// `max_keep==0` means every applied log can be purged.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn calc_purge_upto(&mut self) -> Option<LogId<NID>> {
        let st = &self.state;
        let last_applied = &st.committed;
        let max_keep = self.config.max_applied_log_to_keep;
        let batch_size = self.config.purge_batch_size;

        let mut purge_end = last_applied.next_index().saturating_sub(max_keep);

        if self.config.keep_unsnapshoted_log {
            if let Some(id) = self.snapshot_last_log_id {
                tracing::debug!("the very last log included in snapshot: {}", id);
                // logs not included in snapshot should be kept.
                purge_end = (id.index + 1).min(purge_end);
            } else {
                tracing::debug!("no snapshot, abort purge");
                return None;
            }
        }

        tracing::debug!(
            last_applied = debug(last_applied),
            max_keep,
            "try purge: (-oo, {})",
            purge_end
        );

        if st.last_purged_log_id().next_index() + batch_size > purge_end {
            tracing::debug!(
                last_applied = debug(last_applied),
                max_keep,
                last_purged_log_id = display(st.last_purged_log_id().summary()),
                batch_size,
                purge_end,
                "no need to purge",
            );
            return None;
        }

        let log_id = self.state.log_ids.get(purge_end - 1);
        debug_assert!(
            log_id.is_some(),
            "log id not found at {}, engine.state:{:?}",
            purge_end - 1,
            st
        );

        log_id
    }

    /// Purge log entries upto `upto`, inclusive.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn purge_log(&mut self, upto: LogId<NID>) {
        let st = &mut self.state;
        let log_id = Some(upto);

        if log_id <= st.last_purged_log_id() {
            return;
        }

        st.log_ids.purge(&upto);

        self.push_command(Command::PurgeLog { upto });
    }

    /// Update membership state with a committed membership config
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_committed_membership(&mut self, membership: EffectiveMembership<NID>) {
        tracing::debug!("update committed membership: {}", membership.summary());

        let server_state = self.calc_server_state();

        let m = Arc::new(membership);

        let mut committed = self.state.membership_state.committed.clone();
        let mut effective = self.state.membership_state.effective.clone();

        if committed.log_id < m.log_id {
            committed = m.clone();
        }

        // The local effective membership may conflict with the leader.
        // Thus it has to compare by log-index, e.g.:
        //   membership.log_id       = (10, 5);
        //   local_effective.log_id = (2, 10);
        if effective.log_id.index() <= m.log_id.index() {
            effective = m;
        }

        let mem_state = MembershipState { committed, effective };

        if self.state.membership_state.effective != mem_state.effective {
            self.push_command(Command::UpdateMembership {
                membership: mem_state.effective.clone(),
            })
        }

        self.state.membership_state = mem_state;

        self.update_server_state_if_changed(server_state);
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_effective_membership(&mut self, log_id: &LogId<NID>, m: &Membership<NID>) {
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m.summary());

        self.metrics_flags.set_cluster_changed();

        let server_state = self.calc_server_state();

        let em = Arc::new(EffectiveMembership::new(Some(*log_id), m.clone()));

        self.state.membership_state.effective = em.clone();

        self.push_command(Command::UpdateMembership {
            membership: self.state.membership_state.effective.clone(),
        });

        // If membership changes, the progress should be upgraded.
        if let Some(leader) = &mut self.state.leader {
            let old_progress = leader.progress.clone();

            let old_repls = old_progress.iter().copied().collect::<BTreeMap<_, _>>();

            let learner_ids = em.learner_ids().collect::<Vec<_>>();

            leader.progress = old_progress.upgrade_quorum_set(em, &learner_ids);

            // If it is leader, update replication to reflect membership change.

            let new_repls = leader.progress.iter().copied().collect::<BTreeMap<_, _>>();

            // TODO: test
            let mut add = vec![];
            let mut remove = vec![];
            for (node_id, matched) in new_repls.iter() {
                if !old_repls.contains_key(node_id) {
                    add.push((*node_id, *matched));
                }
            }

            for (node_id, matched) in old_repls.iter() {
                // A leader that is removed will be shut down when this membership log is committed.
                if !new_repls.contains_key(node_id) && node_id != &self.id {
                    remove.push((*node_id, *matched));
                }
            }

            self.push_command(Command::UpdateReplicationStreams { remove, add });
        }

        // Leader should not quit at once.
        // A leader should always keep replicating logs.
        if server_state != ServerState::Leader {
            self.update_server_state_if_changed(server_state);
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_progress(&mut self, node_id: NID, log_id: Option<LogId<NID>>) {
        tracing::debug!("update_progress: node_id:{} log_id:{:?}", node_id, log_id);

        let committed = {
            let leader = match &mut self.state.leader {
                None => {
                    return;
                }
                Some(x) => x,
            };

            tracing::debug!(progress = debug(&leader.progress), "leader progress");

            let res = leader.progress.update(&node_id, log_id);
            match res {
                Ok(c) => *c,
                Err(_) => {
                    // TODO: leader should not append log if it is no longer in the membership.
                    //       There is a chance this will happen:
                    //       If leader is `1`, when a the membership changes from [1,2,3] to [2,3],
                    //       The leader will still try to append log to its local store.
                    //       This is still correct but unnecessary.
                    //       To make thing clear, a leader should stop appending log at once if it is no longer in the
                    //       membership.
                    //       The replication task should be generalized to write log for
                    //       both leader and follower.

                    // unreachable!("updating nonexistent id: {}, progress: {:?}", node_id, leader.progress);

                    return;
                }
            }
        };

        tracing::debug!(committed = debug(&committed), "committed after updating progress");

        // Only when the log id is proposed by current leader, it is committed.
        if let Some(c) = committed {
            if c.leader_id.term != self.state.vote.term || c.leader_id.node_id != self.state.vote.node_id {
                return;
            }
        }

        if let Some(prev_committed) = self.state.update_committed(&committed) {
            self.push_command(Command::ReplicateCommitted {
                committed: self.state.committed,
            });
            self.push_command(Command::LeaderCommit {
                since: prev_committed,
                upto: self.state.committed.unwrap(),
            });
            self.purge_applied_log();
        }
    }

    // --- Draft API ---
    pub fn update_snapshot_last_log(&mut self, id: Option<LogId<NID>>) {
        self.snapshot_last_log_id = id;
    }

    // // --- app API ---
    //
    // /// Write a new log entry.
    // pub(crate) fn write(&mut self) -> Result<Vec<AlgoCmd<NID>>, ForwardToLeader<NID>> {}
    //
    // // --- raft protocol API ---
    //
    // pub(crate) fn handle_install_snapshot() {}
    //
    // pub(crate) fn handle_append_entries_resp() {}
    // pub(crate) fn handle_install_snapshot_resp() {}
}

/// Supporting util
impl<NID: NodeId> Engine<NID> {
    /// Enter leader state.
    ///
    /// Leader state has two phase: election phase and replication phase, similar to paxos phase-1 and phase-2
    fn enter_leader(&mut self) {
        self.state.new_leader();
        // TODO: install heartbeat timer
    }

    /// Leave leader state.
    ///
    /// This node then becomes raft-follower or raft-learner.
    fn leave_leader(&mut self) {
        self.state.leader = None;
        // TODO: install election timer if it is a voter
    }

    /// Update effective membership config if encountering a membership config log entry.
    fn try_update_membership<Ent: RaftEntry<NID>>(&mut self, entry: &Ent) {
        if let Some(m) = entry.get_membership() {
            self.update_effective_membership(entry.get_log_id(), m);
        }
    }

    /// Update membership state if membership config entries are found.
    #[allow(dead_code)]
    fn follower_update_membership<'a, Ent: RaftEntry<NID> + 'a>(
        &mut self,
        entries: impl DoubleEndedIterator<Item = &'a Ent>,
    ) {
        let server_state = self.calc_server_state();

        let memberships = Self::last_two_memberships(entries);
        if memberships.is_empty() {
            return;
        }

        tracing::debug!(
            first = display(memberships.first().summary()),
            "applying new membership configs received from leader"
        );
        tracing::debug!(
            last = display(memberships.last().summary()),
            "applying new membership configs received from leader"
        );

        self.update_membership_state(memberships);
        self.push_command(Command::UpdateMembership {
            membership: self.state.membership_state.effective.clone(),
        });

        self.update_server_state_if_changed(server_state);
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
        tracing::debug!(
            membership_state = debug(&self.state.membership_state),
            "updated membership state"
        );
    }

    fn update_server_state_if_changed(&mut self, prev_server_state: ServerState) {
        let server_state = self.calc_server_state();

        if prev_server_state != server_state {
            self.state.server_state = server_state;
            self.push_command(Command::UpdateServerState { server_state })
        }
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
        if server_state == ServerState::Follower && !self.state.membership_state.effective.is_voter(&self.id) {
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
        if self.state.last_log_id().is_none() && self.state.vote == Vote::default() {
            return Ok(());
        }

        tracing::error!(last_log_id = display(self.state.last_log_id().summary()), ?self.state.vote, "Can not initialize");

        Err(NotAllowed {
            last_log_id: self.state.last_log_id(),
            vote: self.state.vote,
        })
    }

    /// When initialize, the node that accept initialize request has to be a member of the initial config.
    fn check_members_contain_me(&self, m: &Membership<NID>) -> Result<(), NotInMembers<NID>> {
        if !m.is_voter(&self.id) {
            let e = NotInMembers {
                node_id: self.id,
                membership: m.clone(),
            };
            Err(e)
        } else {
            Ok(())
        }
    }

    /// Find the first entry in the input that does not exist on local raft-log,
    /// by comparing the log id.
    fn first_conflicting_index<Ent: RaftLogId<NID>>(&self, entries: &[Ent]) -> usize {
        let l = entries.len();

        for (i, ent) in entries.iter().enumerate() {
            let log_id = ent.get_log_id();
            // for i in 0..l {
            // let log_id = entries[i].get_log_id();

            if !self.state.has_log_id(log_id) {
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

    fn assign_log_ids<'a, Ent: RaftEntry<NID> + 'a>(&mut self, entries: impl Iterator<Item = &'a mut Ent>) {
        let mut log_id = LogId::new(self.state.vote.leader_id(), self.state.last_log_id().next_index());
        for entry in entries {
            entry.set_log_id(&log_id);
            tracing::debug!("assign log id: {}", log_id);
            log_id.index += 1;
        }
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
    pub(crate) fn internal_handle_vote_change(&mut self, vote: &Vote<NID>) -> Result<(), RejectVoteRequest<NID>> {
        // Partial ord compare:
        // Vote does not has to be total ord.
        // `!(a >= b)` does not imply `a < b`.
        if vote >= &self.state.vote {
            // Ok
        } else {
            return Err(RejectVoteRequest::ByVote(self.state.vote));
        }

        tracing::debug!(%vote, "grant vote" );

        // Grant the vote

        // If the vote is granted by a quorum, this node should not try to elect.
        let can_be_leader = !vote.committed;

        // There is an active leader or an active candidate.
        // Do not elect for a while.
        self.push_command(Command::InstallElectionTimer { can_be_leader });
        if vote.committed {
            // There is an active leader, reject election for a while.
            self.push_command(Command::RejectElection {});
        }

        if vote > &self.state.vote {
            self.state.vote = *vote;
            self.push_command(Command::SaveVote { vote: *vote });
        }

        // I'm no longer a leader.
        self.leave_leader();

        #[allow(clippy::collapsible_else_if)]
        if self.state.server_state == ServerState::Follower || self.state.server_state == ServerState::Learner {
            // nothing to do
        } else {
            let node_type = self.state.membership_state.effective.get_node_role(&self.id);
            if node_type == Some(NodeRole::Voter) {
                self.set_server_state(ServerState::Follower);
            } else {
                self.set_server_state(ServerState::Learner);
            }
        }

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn calc_server_state(&self) -> ServerState {
        tracing::debug!(
            is_member = display(self.is_voter()),
            is_leader = display(self.is_leader()),
            is_becoming_leader = display(self.is_becoming_leader()),
            "states"
        );
        if self.is_voter() {
            if self.is_leader() {
                ServerState::Leader
            } else if self.is_becoming_leader() {
                ServerState::Candidate
            } else {
                ServerState::Follower
            }
        } else {
            ServerState::Learner
        }
    }

    fn is_voter(&self) -> bool {
        self.state.membership_state.is_voter(&self.id)
    }

    /// The node is candidate or leader
    fn is_becoming_leader(&self) -> bool {
        // self.state.vote.node_id == self.id
        self.state.leader.is_some()
    }

    fn is_leader(&self) -> bool {
        self.state.vote.node_id == self.id && self.state.vote.committed
    }

    fn push_command(&mut self, cmd: Command<NID>) {
        cmd.update_metrics_flags(&mut self.metrics_flags);
        self.commands.push(cmd)
    }
}
