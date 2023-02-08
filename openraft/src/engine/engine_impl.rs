use std::sync::Arc;

use crate::core::ServerState;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::handler::vote_handler::VoteHandler;
use crate::engine::Command;
use crate::entry::RaftEntry;
use crate::error::InitializeError;
use crate::error::NotAMembershipEntry;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::error::RejectVoteRequest;
use crate::internal_server_state::InternalServerState;
use crate::leader::Leader;
use crate::membership::EffectiveMembership;
use crate::membership::NodeRole;
use crate::node::Node;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::raft::AppendEntriesResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::LogStateReader;
use crate::raft_state::RaftState;
use crate::raft_types::RaftLogId;
use crate::summary::MessageSummary;
use crate::validate::Valid;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::NodeId;
use crate::SnapshotMeta;
use crate::Vote;

/// Config for Engine
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct EngineConfig<NID: NodeId> {
    /// The id of this node.
    pub(crate) id: NID,

    /// The maximum number of applied logs to keep before purging.
    pub(crate) max_in_snapshot_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    pub(crate) purge_batch_size: u64,

    /// The maximum number of entries per payload allowed to be transmitted during replication
    pub(crate) max_payload_entries: u64,
}

impl<NID: NodeId> Default for EngineConfig<NID> {
    fn default() -> Self {
        Self {
            id: NID::default(),
            max_in_snapshot_log_to_keep: 1000,
            purge_batch_size: 256,
            max_payload_entries: 300,
        }
    }
}

/// The entry of output from Engine to the runtime.
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct EngineOutput<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Tracks what kind of metrics changed
    pub(crate) metrics_flags: MetricsChangeFlags,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: Vec<Command<NID, N>>,
}

impl<NID, N> EngineOutput<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) fn push_command(&mut self, cmd: Command<NID, N>) {
        cmd.update_metrics_flags(&mut self.metrics_flags);
        self.commands.push(cmd)
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
pub(crate) struct Engine<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: EngineConfig<NID>,

    /// The state of this raft node.
    pub(crate) state: Valid<RaftState<NID, N>>,

    /// The internal server state used by Engine.
    pub(crate) internal_server_state: InternalServerState<NID>,

    /// Output entry for the runtime.
    pub(crate) output: EngineOutput<NID, N>,
}

impl<NID, N> Engine<NID, N>
where
    N: Node,
    NID: NodeId,
{
    pub(crate) fn new(init_state: RaftState<NID, N>, config: EngineConfig<NID>) -> Self {
        Self {
            config,
            state: Valid::new(init_state),
            internal_server_state: InternalServerState::default(),
            output: EngineOutput::default(),
        }
    }

    // TODO: test it
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn startup(&mut self) {
        // Allows starting up as a leader.

        // Previously it is a leader. restore it as leader at once
        if self.state.is_leader(&self.config.id) {
            self.switch_internal_server_state();
            self.update_server_state_if_changed();

            let mut rh = self.replication_handler();
            rh.update_replication_streams();
            rh.initiate_replication();

            return;
        }

        let server_state = if self.state.membership_state.effective().is_voter(&self.config.id) {
            ServerState::Follower
        } else {
            ServerState::Learner
        };

        self.state.server_state = server_state;

        tracing::debug!(
            "startup: id={} target_state: {:?}",
            self.config.id,
            self.state.server_state
        );
    }

    /// Initialize a node by appending the first log.
    ///
    /// - The first log has to be membership config log.
    /// - The node has to contain no logs at all and the vote is the minimal value. See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    ///
    /// Appending the very first log is slightly different from appending log by a leader or follower.
    /// This step is not confined by the consensus protocol and has to be dealt with differently.
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn initialize<Ent: RaftEntry<NID, N>>(
        &mut self,
        entries: &mut [Ent],
    ) -> Result<(), InitializeError<NID, N>> {
        let l = entries.len();
        debug_assert_eq!(1, l);

        self.check_initialize()?;

        self.assign_log_ids(entries.iter_mut());
        self.state.extend_log_ids_from_same_leader(entries);

        self.output.push_command(Command::AppendInputEntries { range: 0..l });

        let entry = &mut entries[0];
        if let Some(m) = entry.get_membership() {
            self.check_members_contain_me(m)?;
        } else {
            Err(NotAMembershipEntry {})?;
        }
        self.try_update_membership(entry);

        self.output.push_command(Command::MoveInputCursorBy { n: l });

        // With the new config, start to elect to become leader
        self.elect();

        Ok(())
    }

    /// Start to elect this node as leader
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn elect(&mut self) {
        self.handle_vote_change(&Vote::new(self.state.vote.term + 1, self.config.id)).unwrap();

        // Safe unwrap()
        let leader = self.internal_server_state.leading_mut().unwrap();
        leader.grant_vote_by(self.config.id);
        let quorum_granted = leader.is_vote_granted();

        // Fast-path: if there is only one node in the cluster.

        if quorum_granted {
            self.establish_leader();
            return;
        }

        // Slow-path: send vote request, let a quorum grant it.

        self.output.push_command(Command::SendVote {
            vote_req: VoteRequest::new(self.state.vote, self.state.last_log_id().copied()),
        });

        // TODO: For compatibility. remove it. The runtime does not need to know about server state.
        self.update_server_state_if_changed();
        self.output.push_command(Command::InstallElectionTimer { can_be_leader: true });
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<NID>) -> VoteResponse<NID> {
        tracing::debug!(req = display(req.summary()), "Engine::handle_vote_req");
        tracing::debug!(
            my_vote = display(self.state.vote.summary()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            "Engine::handle_vote_req"
        );

        let res = if req.last_log_id.as_ref() >= self.state.last_log_id() {
            self.handle_vote_change(&req.vote)
        } else {
            Err(RejectVoteRequest::ByLastLogId(self.state.last_log_id().copied()))
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
            last_log_id: self.state.last_log_id().copied(),
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
        let leader = match &mut self.internal_server_state {
            InternalServerState::Leading(l) => l,
            InternalServerState::Following => return,
        };

        if resp.vote < self.state.vote {
            debug_assert!(!resp.vote_granted);
        }

        if resp.vote_granted {
            leader.grant_vote_by(target);

            let quorum_granted = leader.is_vote_granted();
            if quorum_granted {
                tracing::debug!("quorum granted vote");
                self.establish_leader();
            }
            return;
        }

        // vote is rejected:

        debug_assert_eq!(
            Some(NodeRole::Voter),
            self.state.membership_state.effective().get_node_role(&self.config.id)
        );

        // If peer's vote is greater than current vote, revert to follower state.
        if resp.vote > self.state.vote {
            self.state.vote = resp.vote;
            self.output.push_command(Command::SaveVote { vote: self.state.vote });
        }

        // Seen a higher log.
        // TODO: if already installed a timer with can_be_leader==false, it should not install a timer with
        //       can_be_leader==true.
        if resp.last_log_id.as_ref() > self.state.last_log_id() {
            self.output.push_command(Command::InstallElectionTimer { can_be_leader: false });
        } else {
            self.output.push_command(Command::InstallElectionTimer { can_be_leader: true });
        }

        debug_assert!(self.state.is_voter(&self.config.id));

        // When vote is rejected, it does not need to leave candidate state.
        // Candidate loop, follower loop and learner loop are totally the same.
        //
        // The only thing that needs to do is update election timer.
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
    pub(crate) fn leader_append_entries<'a, Ent: RaftEntry<NID, N> + 'a>(&mut self, entries: &mut [Ent]) {
        let l = entries.len();
        if l == 0 {
            return;
        }

        self.assign_log_ids(entries.iter_mut());
        self.state.extend_log_ids_from_same_leader(entries);

        self.output.push_command(Command::AppendInputEntries { range: 0..l });

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
                    let mut rh = self.replication_handler();
                    if let Some(prev_log_id) = rh.state.get_log_id(log_index - 1) {
                        let id = rh.config.id;

                        // The leader may not be in membership anymore
                        if rh.leader.progress.index(&id).is_some() {
                            let inflight_id = {
                                let prog_entry = rh.leader.progress.get_mut(&id).unwrap();
                                // TODO: It should be self.state.last_log_id() but None is ok.
                                prog_entry.inflight = Inflight::logs(None, Some(prev_log_id));
                                prog_entry.inflight.get_id().unwrap()
                            };
                            rh.update_matching(id, inflight_id, Some(prev_log_id));
                        }
                    }
                }

                // since this entry, the condition to commit has been changed.
                self.update_effective_membership(entry.get_log_id(), _m);
            }
        }

        let mut rh = self.replication_handler();
        {
            // Safe unwrap(): entries.len() > 0
            let last = entries.last().unwrap();
            let id = rh.config.id;

            // The leader may not be in membership anymore
            if rh.leader.progress.index(&id).is_some() {
                let inflight_id = {
                    let prog_entry = rh.leader.progress.get_mut(&id).unwrap();
                    // TODO: It should be self.state.last_log_id() but None is ok.
                    prog_entry.inflight = Inflight::logs(None, Some(*last.get_log_id()));
                    prog_entry.inflight.get_id().unwrap()
                };
                rh.update_matching(id, inflight_id, Some(*last.get_log_id()));
            }
        }

        // Still need to replicate to learners, even when it is fast-committed.
        rh.initiate_replication();

        self.output.push_command(Command::MoveInputCursorBy { n: l });
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
        Ent: RaftEntry<NID, N> + MessageSummary<Ent> + 'a,
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
            my_committed = display(self.state.committed().summary()),
            "local state"
        );

        let res = self.handle_vote_change(vote);
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
            committed = display(self.state.committed().summary()),
            entries = display(entries.summary()),
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
    pub(crate) fn follower_commit_entries<'a, Ent: RaftEntry<NID, N> + 'a>(
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
            self.output.push_command(Command::FollowerCommit {
                // TODO(xp): when restart, commit is reset to None. Use last_applied instead.
                already_committed: prev_committed,
                upto: committed.unwrap(),
            });
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
    pub(crate) fn follower_do_append_entries<'a, Ent: RaftEntry<NID, N> + 'a>(
        &mut self,
        entries: &[Ent],
        since: usize,
    ) {
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

        self.output.push_command(Command::AppendInputEntries { range: since..l });
        self.follower_update_membership(entries.iter());

        // TODO(xp): should be moved to handle_append_entries_req()
        self.output.push_command(Command::MoveInputCursorBy { n: l });
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

        self.output.push_command(Command::DeleteConflictLog { since: since_log_id });

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

        let effective = self.state.membership_state.effective().clone();
        if Some(since) <= effective.log_id.index() {
            let committed = self.state.membership_state.committed().clone();

            tracing::debug!(
                effective = debug(&effective),
                committed = debug(&committed),
                "effective membership is in conflicting logs, revert it to last committed"
            );

            debug_assert!(
                committed.log_id.index() < Some(since),
                "committed membership can not conflict with the leader"
            );

            let mem_state = MembershipState::new(committed.clone(), committed);

            self.state.membership_state = mem_state;
            self.output.push_command(Command::UpdateMembership {
                membership: self.state.membership_state.effective().clone(),
            });

            tracing::debug!(effective = debug(&effective), "Done reverting membership");

            self.update_server_state_if_changed();
        }
    }

    /// Update membership state with a committed membership config
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_committed_membership(&mut self, membership: EffectiveMembership<NID, N>) {
        tracing::debug!("update committed membership: {}", membership.summary());

        let m = Arc::new(membership);

        // TODO: if effective membership changes, call `update_repliation()`
        let effective_changed = self.state.membership_state.update_committed(m);
        if let Some(c) = effective_changed {
            self.output.push_command(Command::UpdateMembership { membership: c })
        }

        self.update_server_state_if_changed();
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_effective_membership(&mut self, log_id: &LogId<NID>, m: &Membership<NID, N>) {
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m.summary());

        let em = Arc::new(EffectiveMembership::new(Some(*log_id), m.clone()));

        self.state.membership_state.set_effective(em.clone());

        self.output.push_command(Command::UpdateMembership {
            membership: self.state.membership_state.effective().clone(),
        });

        let end = self.state.last_log_id().next_index();

        // If membership changes, the progress should be upgraded.
        if let Some(leader) = &mut self.internal_server_state.leading_mut() {
            let old_progress = leader.progress.clone();
            let learner_ids = em.learner_ids().collect::<Vec<_>>();

            leader.progress =
                old_progress.upgrade_quorum_set(em.membership.to_quorum_set(), &learner_ids, ProgressEntry::empty(end));
        }

        // A leader that is removed will be shut down when this membership log is committed.
        // TODO(9): currently only a leader has replication setup.
        //       It's better to setup replication for both leader and candidate.
        //       e.g.: if self.internal_server_state.is_leading() {
        if self.state.is_leader(&self.config.id) {
            let mut rh = self.replication_handler();
            rh.update_replication_streams();
            rh.initiate_replication();
        }

        // Leader should not quit at once.
        // A leader should always keep replicating logs.
        if self.state.server_state != ServerState::Leader {
            self.update_server_state_if_changed();
        }
    }

    /// Leader steps down(convert to learner) once the membership not containing it is committed.
    ///
    /// This is only called by leader.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn leader_step_down(&mut self) {
        tracing::debug!("leader_step_down: node_id:{}", self.config.id);

        // Step down:
        // Keep acting as leader until a membership without this node is committed.
        let em = &self.state.membership_state.effective();

        tracing::debug!(
            "membership: {}, committed: {}, is_leading: {}",
            em.summary(),
            self.state.committed().summary(),
            self.state.is_leading(&self.config.id),
        );

        #[allow(clippy::collapsible_if)]
        if em.log_id.as_ref() <= self.state.committed() {
            if !em.is_voter(&self.config.id) && self.state.is_leading(&self.config.id) {
                tracing::debug!("leader {} is stepping down", self.config.id);
                self.enter_following();
            }
        }
    }

    /// Follower/Learner handles install-snapshot.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn install_snapshot(&mut self, meta: SnapshotMeta<NID, N>) {
        // There are two special cases in which snapshot last log id does not exists locally:
        // Snapshot last log id before the local last-purged-log-id, or after the local last-log-id:
        //
        //      snapshot ----.
        //                   v
        // -----------------------llllllllll--->
        //
        //      snapshot ----.
        //                   v
        // ----lllllllllll--------------------->
        //
        // In the first case, snapshot-last-log-id <= last-purged-log-id <= local-snapshot-last-log-id.
        // Thus snapshot is obsolete and won't be installed.
        //
        // In the second case, all local logs will be purged after install.

        tracing::info!("install_snapshot: meta:{:?}", meta);

        // TODO: temp solution: committed is updated after snapshot_last_log_id.
        //       committed should be updated first or together with snapshot_last_log_id(i.e., extract `state` first).
        let old_validate = self.state.enable_validate;
        self.state.enable_validate = false;

        let snap_last_log_id = meta.last_log_id;

        if snap_last_log_id.as_ref() <= self.state.committed() {
            tracing::info!(
                "No need to install snapshot; snapshot last_log_id({}) <= committed({})",
                snap_last_log_id.summary(),
                self.state.committed().summary()
            );
            self.output.push_command(Command::CancelSnapshot { snapshot_meta: meta });
            // TODO: temp solution: committed is updated after snapshot_last_log_id.
            self.state.enable_validate = old_validate;
            return;
        }

        // snapshot_last_log_id can not be None
        let snap_last_log_id = snap_last_log_id.unwrap();

        let mut snap_handler = self.snapshot_handler();
        let updated = snap_handler.update_snapshot(meta.clone());
        if !updated {
            // TODO: temp solution: committed is updated after snapshot_last_log_id.
            self.state.enable_validate = old_validate;
            return;
        }

        // Do install:
        // 1. Truncate all logs if conflict
        //    Unlike normal append-entries RPC, if conflicting logs are found, it is not **necessary** to delete them.
        //    But cleaning them make the assumption of incremental-log-id always hold, which makes it easier to debug.
        //    See: [Snapshot-replication](https://datafuselabs.github.io/openraft/replication.html#snapshot-replication)
        //
        //    Truncate all:
        //
        //    It just truncate **ALL** logs here, because `snap_last_log_id` is committed, if the local log id conflicts
        //    with `snap_last_log_id`, there must be a quorum that contains `snap_last_log_id`.
        //    Thus it is safe to remove all logs on this node.
        //
        //    The logs before `snap_last_log_id` may conflicts with the leader too.
        //    It's not safe to remove the conflicting logs that are less than `snap_last_log_id` after installing
        //    snapshot.
        //
        //    If the node crashes, dirty logs may remain there. These logs may be forwarded to other nodes if this nodes
        //    becomes a leader.
        //
        // 2. Install snapshot.

        let local = self.state.get_log_id(snap_last_log_id.index);
        if let Some(local) = local {
            if local != snap_last_log_id {
                // Delete non-committed logs.
                self.truncate_logs(self.state.committed().next_index());
            }
        }

        self.state.committed = Some(snap_last_log_id);
        self.update_committed_membership(meta.last_membership.clone());

        // TODO: There should be two separate commands for installing snapshot:
        //       - Replace state machine with snapshot and replace the `current_snapshot` in the store.
        //       - Do not install, just replace the `current_snapshot` with a newer one. This command can be used for
        //         leader to synchronize its snapshot data.
        self.output.push_command(Command::InstallSnapshot { snapshot_meta: meta });

        // A local log that is <= snap_last_log_id can not conflict with the leader.
        // But there will be a hole in the logs. Thus it's better remove all logs.

        // In the second case, if local-last-log-id is smaller than snapshot-last-log-id,
        // and this node crashes after installing snapshot and before purging logs,
        // the log will be purged the next start up, in [`RaftState::get_initial_state`].
        // TODO: move this to LogHandler::purge_log()?
        self.state.purge_upto = Some(snap_last_log_id);
        self.log_handler().purge_log();

        // TODO: temp solution: committed is updated after snapshot_last_log_id.
        self.state.enable_validate = old_validate;
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn finish_building_snapshot(&mut self, meta: SnapshotMeta<NID, N>) {
        tracing::info!("finish_building_snapshot: {:?}", meta);

        let mut h = self.snapshot_handler();

        let updated = h.update_snapshot(meta);
        if !updated {
            return;
        }

        self.log_handler().update_purge_upto();

        if self.internal_server_state.is_leading() {
            // If it is leading, it must not delete a log that is in use by a replication task.
            self.replication_handler().try_purge_log();
        } else {
            // For follower/learner, no other tasks are using logs, just purge.
            self.log_handler().purge_log();
        }
    }
}

/// Supporting util
impl<NID, N> Engine<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Enter leading or following state by checking `vote`.
    ///
    /// `vote.node_id == self.id`: Leading state;
    /// `vote.node_id != self.id`: Following state;
    pub(crate) fn switch_internal_server_state(&mut self) {
        if self.state.vote.node_id == self.config.id {
            self.enter_leading();
        } else {
            self.enter_following();
        }
    }

    /// Enter leading state(vote.node_id == self.id) .
    ///
    /// Leader state has two phase: election phase and replication phase, similar to paxos phase-1 and phase-2
    pub(crate) fn enter_leading(&mut self) {
        debug_assert_eq!(self.state.vote.node_id, self.config.id);
        // debug_assert!(self.internal_server_state.is_following(), "can not enter leading twice");

        self.new_leading();
    }

    /// Leave leading state and enter following state(vote.node_id != self.id).
    ///
    /// This node then becomes raft-follower or raft-learner.
    pub(crate) fn enter_following(&mut self) {
        // TODO: entering following needs to check last-log-id on other node to decide the election timeout.

        // TODO: a candidate that can not elect successfully should not enter following state.
        //       It should just sleep in leading state(candidate state for an application).
        //       This way it holds that 'vote.node_id != self.id <=> following state`.
        // debug_assert_ne!(self.state.vote.node_id, self.id);

        // debug_assert!(
        //     self.internal_server_state.is_leading(),
        //     "can not enter following twice"
        // );

        let vote = &self.state.vote;

        // TODO: installing election timer should be driven by change of last-log-id
        if vote.committed {
            // There is an active leader.
            // Do not elect for a longer while.
            // TODO: Installing a timer should not be part of the Engine's job.
            self.output.push_command(Command::InstallElectionTimer { can_be_leader: false });
        } else {
            // There is an active candidate.
            // Do not elect for a short while.
            self.output.push_command(Command::InstallElectionTimer { can_be_leader: true });
        }

        if self.internal_server_state.is_following() {
            return;
        }

        self.internal_server_state = InternalServerState::Following;

        self.update_server_state_if_changed();
    }

    /// Vote is granted by a quorum, leader established.
    #[tracing::instrument(level = "debug", skip_all)]
    fn establish_leader(&mut self) {
        self.vote_handler().commit();

        self.update_server_state_if_changed();
        self.replication_handler().update_replication_streams();

        // Only when a log with current `vote` is replicated to a quorum, the logs are considered committed.
        self.append_blank_log();

        // Send former logs and the blank log.
        self.replication_handler().initiate_replication();
    }

    // TODO: Incomplete method:
    //       This should ony be used by test and internally.
    //       No other mod should use it
    /// Create a new Leader, when raft enters candidate state.
    /// In openraft, Leader and Candidate shares the same state.
    pub(crate) fn new_leading(&mut self) {
        let em = &self.state.membership_state.effective();
        let mut leader = Leader::new(
            em.membership.to_quorum_set(),
            em.learner_ids(),
            self.state.last_log_id().index(),
        );

        // TODO: for a new leader, update the matching
        // We can just ignore the result here:
        // The `committed` will not be updated until a log of current term is granted by a quorum
        let _ = leader.progress.update_with(&self.config.id, |v| v.matching = self.state.last_log_id().copied());

        self.internal_server_state = InternalServerState::Leading(leader);
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn append_blank_log(&mut self) {
        let log_id = LogId {
            leader_id: self.state.vote.leader_id(),
            index: self.state.last_log_id().next_index(),
        };
        self.state.log_ids.append(log_id);
        self.output.push_command(Command::AppendBlankLog { log_id });

        let id = self.config.id;
        let mut rh = self.replication_handler();
        {
            // leader must initialize its replication progress too.
            // TODO: refactor this
            let prog_entry = rh.leader.progress.get_mut(&id).unwrap();
            let res = prog_entry.next_send(rh.state, 1).unwrap();
            let inflight_id = res.get_id().unwrap();
            debug_assert_eq!(
                &Inflight::logs(rh.state.get_log_id(log_id.index - 1), Some(log_id)).with_id(inflight_id),
                res
            );
            rh.update_matching(id, inflight_id, Some(log_id));
        }
    }

    /// Update effective membership config if encountering a membership config log entry.
    fn try_update_membership<Ent: RaftEntry<NID, N>>(&mut self, entry: &Ent) {
        if let Some(m) = entry.get_membership() {
            self.update_effective_membership(entry.get_log_id(), m);
        }
    }

    /// Update membership state if membership config entries are found.
    #[allow(dead_code)]
    fn follower_update_membership<'a, Ent: RaftEntry<NID, N> + 'a>(
        &mut self,
        entries: impl DoubleEndedIterator<Item = &'a Ent>,
    ) {
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
        self.output.push_command(Command::UpdateMembership {
            membership: self.state.membership_state.effective().clone(),
        });

        self.update_server_state_if_changed();
    }

    /// Find the last 2 membership entries in a list of entries.
    ///
    /// A follower/learner reverts the effective membership to the previous one,
    /// when conflicting logs are found.
    ///
    /// See: [Effective-membership](https://datafuselabs.github.io/openraft/effective-membership.html)
    fn last_two_memberships<'a, Ent: RaftEntry<NID, N> + 'a>(
        entries: impl DoubleEndedIterator<Item = &'a Ent>,
    ) -> Vec<EffectiveMembership<NID, N>> {
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
    fn update_membership_state(&mut self, memberships: Vec<EffectiveMembership<NID, N>>) {
        debug_assert!(self.state.membership_state.effective().log_id < memberships[0].log_id);

        let new_mem_state = if memberships.len() == 1 {
            MembershipState::new(
                self.state.membership_state.effective().clone(),
                Arc::new(memberships[0].clone()),
            )
        } else {
            // len() == 2
            MembershipState::new(Arc::new(memberships[0].clone()), Arc::new(memberships[1].clone()))
        };
        self.state.membership_state = new_mem_state;
        tracing::debug!(
            membership_state = debug(&self.state.membership_state),
            "updated membership state"
        );
    }

    fn update_server_state_if_changed(&mut self) {
        let server_state = self.state.calc_server_state(&self.config.id);

        tracing::debug!(
            id = display(self.config.id),
            prev_server_state = debug(self.state.server_state),
            server_state = debug(server_state),
            "update_server_state_if_changed"
        );

        if self.state.server_state == server_state {
            return;
        }

        let was_leader = self.state.server_state == ServerState::Leader;
        let is_leader = server_state == ServerState::Leader;

        if !was_leader && is_leader {
            self.output.push_command(Command::BecomeLeader);
        } else if was_leader && !is_leader {
            self.output.push_command(Command::QuitLeader);
        } else {
            // nothing to do
        }

        self.state.server_state = server_state;
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
            last_log_id: self.state.last_log_id().copied(),
            vote: self.state.vote,
        })
    }

    /// When initialize, the node that accept initialize request has to be a member of the initial config.
    fn check_members_contain_me(&self, m: &Membership<NID, N>) -> Result<(), NotInMembers<NID, N>> {
        if !m.is_voter(&self.config.id) {
            let e = NotInMembers {
                node_id: self.config.id,
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

    fn assign_log_ids<'a, Ent: RaftEntry<NID, N> + 'a>(&mut self, entries: impl Iterator<Item = &'a mut Ent>) {
        let mut log_id = LogId::new(self.state.vote.leader_id(), self.state.last_log_id().next_index());
        for entry in entries {
            entry.set_log_id(&log_id);
            tracing::debug!("assign log id: {}", log_id);
            log_id.index += 1;
        }
    }

    /// Check and change vote.
    /// This is used by all 3 RPC append-entries, vote, install-snapshot to check the `vote` field.
    ///
    /// Grant vote if vote >= mine.
    /// Note: This method does not check last-log-id. handle-vote-request has to deal with last-log-id itself.
    pub(crate) fn handle_vote_change(&mut self, vote: &Vote<NID>) -> Result<(), RejectVoteRequest<NID>> {
        // Partial ord compare:
        // Vote does not has to be total ord.
        // `!(a >= b)` does not imply `a < b`.
        if vote >= &self.state.vote {
            // Ok
        } else {
            return Err(RejectVoteRequest::ByVote(self.state.vote));
        }
        tracing::debug!(%vote, "vote is changing to" );

        // Grant the vote

        if vote > &self.state.vote {
            self.state.vote = *vote;
            self.output.push_command(Command::SaveVote { vote: *vote });
        }

        self.switch_internal_server_state();

        Ok(())
    }

    // Only used by tests
    #[allow(dead_code)]
    pub(crate) fn calc_server_state(&self) -> ServerState {
        self.state.calc_server_state(&self.config.id)
    }

    // --- handlers ---

    pub(crate) fn vote_handler(&mut self) -> VoteHandler<NID, N> {
        VoteHandler {
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<NID, N> {
        LogHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn snapshot_handler(&mut self) -> SnapshotHandler<NID, N> {
        SnapshotHandler {
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<NID, N> {
        let leader = match self.internal_server_state.leading_mut() {
            None => {
                unreachable!("There is no leader, can not handle replication");
            }
            Some(x) => x,
        };

        ReplicationHandler {
            config: &mut self.config,
            leader,
            state: &mut self.state,
            output: &mut self.output,
        }
    }
}
