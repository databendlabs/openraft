use std::sync::Arc;

use crate::display_ext::DisplayOption;
use crate::display_ext::DisplaySlice;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::entry::RaftEntry;
use crate::raft::AppendEntriesResponse;
use crate::raft_state::LogStateReader;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::SnapshotMeta;
use crate::StoredMembership;

#[cfg(test)] mod append_entries_test;
#[cfg(test)] mod commit_entries_test;
#[cfg(test)] mod do_append_entries_test;
#[cfg(test)] mod install_snapshot_test;
#[cfg(test)] mod truncate_logs_test;
#[cfg(test)] mod update_committed_membership_test;

/// Receive replication request and deal with them.
///
/// It mainly implements the logic of a follower/learner
pub(crate) struct FollowingHandler<'x, NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    pub(crate) config: &'x mut EngineConfig<NID>,
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N, Ent>,
}

impl<'x, NID, N, Ent> FollowingHandler<'x, NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_entries(
        &mut self,
        prev_log_id: Option<LogId<NID>>,
        entries: Vec<Ent>,
        leader_committed: Option<LogId<NID>>,
    ) -> AppendEntriesResponse<NID> {
        tracing::debug!(
            prev_log_id = display(prev_log_id.summary()),
            entries = display(DisplaySlice::<_>(&entries)),
            leader_committed = display(leader_committed.summary()),
            "append-entries request"
        );
        tracing::debug!(
            my_last_log_id = display(self.state.last_log_id().summary()),
            my_committed = display(self.state.committed().summary()),
            "local state"
        );

        if let Some(x) = entries.first() {
            debug_assert!(x.get_log_id().index == prev_log_id.next_index());
        }

        if let Some(ref prev) = prev_log_id {
            if !self.state.has_log_id(prev) {
                let local = self.state.get_log_id(prev.index);
                tracing::debug!(local = display(local.summary()), "prev_log_id does not match");

                self.truncate_logs(prev.index);
                return AppendEntriesResponse::Conflict;
            }
        }

        // else `prev_log_id.is_none()` means replicating logs from the very beginning.

        tracing::debug!(
            committed = display(self.state.committed().summary()),
            entries = display(DisplaySlice::<_>(&entries)),
            "prev_log_id matches, skip matching entries",
        );

        let last_log_id = entries.last().map(|x| *x.get_log_id());

        self.state.update_accepted(std::cmp::max(prev_log_id, last_log_id));

        let l = entries.len();
        let since = self.state.first_conflicting_index(&entries);
        if since < l {
            // Before appending, if an entry overrides an conflicting one,
            // the entries after it has to be deleted first.
            // Raft requires log ids are in total order by (term,index).
            // Otherwise the log id with max index makes committed entry invisible in election.
            self.truncate_logs(entries[since].get_log_id().index);
        }

        self.do_append_entries(entries, since);

        self.commit_entries(leader_committed);

        AppendEntriesResponse::Success
    }

    /// Follower/Learner appends `entries[since..]`.
    ///
    /// It assumes:
    /// - Previous entries all match the leader's.
    /// - conflicting entries are deleted.
    ///
    /// Membership config changes are also detected and applied here.
    #[tracing::instrument(level = "debug", skip(self, entries))]
    fn do_append_entries(&mut self, mut entries: Vec<Ent>, since: usize) {
        let l = entries.len();

        if since == l {
            return;
        }

        let entries = entries.split_off(since);

        debug_assert_eq!(
            entries[0].get_log_id().index,
            self.state.log_ids.last().cloned().next_index(),
        );

        debug_assert!(Some(entries[0].get_log_id()) > self.state.log_ids.last());

        self.state.extend_log_ids(&entries);
        self.append_membership(entries.iter());

        self.output.push_command(Command::AppendInputEntries { entries });
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn commit_entries(&mut self, leader_committed: Option<LogId<NID>>) {
        let accepted = self.state.accepted().copied();
        let committed = std::cmp::min(accepted, leader_committed);

        tracing::debug!(
            leader_committed = display(DisplayOption(&leader_committed)),
            accepted = display(DisplayOption(&accepted)),
            committed = display(DisplayOption(&committed)),
            "{}",
            func_name!()
        );

        if let Some(prev_committed) = self.state.update_committed(&committed) {
            self.output.push_command(Command::FollowerCommit {
                // TODO(xp): when restart, commit is reset to None. Use last_applied instead.
                already_committed: prev_committed,
                upto: committed.unwrap(),
            });
        }
    }

    /// Delete log entries since log index `since`, inclusive, when the log at `since` is found
    /// conflict with the leader.
    ///
    /// And revert effective membership to the last committed if it is from the conflicting logs.
    #[tracing::instrument(level = "debug", skip(self))]
    fn truncate_logs(&mut self, since: u64) {
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

        let changed = self.state.membership_state.truncate(since);
        if let Some(c) = changed {
            self.output.push_command(Command::UpdateMembership { membership: c });
            self.server_state_handler().update_server_state_if_changed();
        }
    }

    /// Append membership log if membership config entries are found, after appending entries to
    /// log.
    fn append_membership<'a>(&mut self, entries: impl DoubleEndedIterator<Item = &'a Ent>)
    where Ent: 'a {
        let memberships = Self::last_two_memberships(entries);
        if memberships.is_empty() {
            return;
        }

        // Update membership state with the last 2 membership configs found in new log entries.
        // Other membership log can be just ignored.
        for (i, m) in memberships.into_iter().enumerate() {
            tracing::debug!(
                last = display(m.summary()),
                "applying {}-th new membership configs received from leader",
                i
            );
            self.state.membership_state.append(Arc::new(EffectiveMembership::new_from_stored_membership(m)));
        }

        tracing::debug!(
            membership_state = display(&self.state.membership_state.summary()),
            "updated membership state"
        );

        self.output.push_command(Command::UpdateMembership {
            membership: self.state.membership_state.effective().clone(),
        });

        self.server_state_handler().update_server_state_if_changed();
    }

    /// Update membership state with a committed membership config
    #[tracing::instrument(level = "debug", skip_all)]
    fn update_committed_membership(&mut self, membership: EffectiveMembership<NID, N>) {
        tracing::debug!("update committed membership: {}", membership.summary());

        let m = Arc::new(membership);

        // TODO: if effective membership changes, call `update_repliation()`
        let effective_changed = self.state.membership_state.update_committed(m);
        if let Some(c) = effective_changed {
            self.output.push_command(Command::UpdateMembership { membership: c })
        }

        self.server_state_handler().update_server_state_if_changed();
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
        // In the first case, snapshot-last-log-id <= last-purged-log-id <=
        // local-snapshot-last-log-id. Thus snapshot is obsolete and won't be installed.
        //
        // In the second case, all local logs will be purged after install.

        tracing::info!("install_snapshot: meta:{:?}", meta);

        // // TODO: temp solution: committed is updated after snapshot_last_log_id.
        // //       committed should be updated first or together with snapshot_last_log_id(i.e.,
        // extract `state` first). let old_validate = self.state.enable_validate;
        // self.state.enable_validate = false;

        let snap_last_log_id = meta.last_log_id;

        if snap_last_log_id.as_ref() <= self.state.committed() {
            tracing::info!(
                "No need to install snapshot; snapshot last_log_id({}) <= committed({})",
                snap_last_log_id.summary(),
                self.state.committed().summary()
            );
            self.output.push_command(Command::CancelSnapshot { snapshot_meta: meta });
            // // TODO: temp solution: committed is updated after snapshot_last_log_id.
            // self.state.enable_validate = old_validate;
            return;
        }

        // snapshot_last_log_id can not be None
        let snap_last_log_id = snap_last_log_id.unwrap();

        let mut snap_handler = self.snapshot_handler();
        let updated = snap_handler.update_snapshot(meta.clone());
        if !updated {
            // // TODO: temp solution: committed is updated after snapshot_last_log_id.
            // self.state.enable_validate = old_validate;
            return;
        }

        // Do install:
        // 1. Truncate all logs if conflict
        //    Unlike normal append-entries RPC, if conflicting logs are found, it is not
        // **necessary** to delete them.    But cleaning them make the assumption of
        // incremental-log-id always hold, which makes it easier to debug.    See: [Snapshot-replication](https://datafuselabs.github.io/openraft/replication.html#snapshot-replication)
        //
        //    Truncate all:
        //
        //    It just truncate **ALL** logs here, because `snap_last_log_id` is committed, if the
        // local log id conflicts    with `snap_last_log_id`, there must be a quorum that
        // contains `snap_last_log_id`.    Thus it is safe to remove all logs on this node.
        //
        //    The logs before `snap_last_log_id` may conflicts with the leader too.
        //    It's not safe to remove the conflicting logs that are less than `snap_last_log_id`
        // after installing    snapshot.
        //
        //    If the node crashes, dirty logs may remain there. These logs may be forwarded to other
        // nodes if this nodes    becomes a leader.
        //
        // 2. Install snapshot.

        let local = self.state.get_log_id(snap_last_log_id.index);
        if let Some(local) = local {
            if local != snap_last_log_id {
                // Delete non-committed logs.
                self.truncate_logs(self.state.committed().next_index());
            }
        }

        self.state.update_accepted(Some(snap_last_log_id));
        self.state.committed = Some(snap_last_log_id);
        self.update_committed_membership(EffectiveMembership::new_from_stored_membership(
            meta.last_membership.clone(),
        ));

        // TODO: There should be two separate commands for installing snapshot:
        //       - Replace state machine with snapshot and replace the `current_snapshot` in the store.
        //       - Do not install, just replace the `current_snapshot` with a newer one. This command can be
        //         used for leader to synchronize its snapshot data.
        self.output.push_command(Command::InstallSnapshot { snapshot_meta: meta });

        // A local log that is <= snap_last_log_id can not conflict with the leader.
        // But there will be a hole in the logs. Thus it's better remove all logs.

        // In the second case, if local-last-log-id is smaller than snapshot-last-log-id,
        // and this node crashes after installing snapshot and before purging logs,
        // the log will be purged the next start up, in [`RaftState::get_initial_state`].
        // TODO: move this to LogHandler::purge_log()?
        self.state.purge_upto = Some(snap_last_log_id);
        self.log_handler().purge_log();

        // // TODO: temp solution: committed is updated after snapshot_last_log_id.
        // self.state.enable_validate = old_validate;
    }

    /// Find the last 2 membership entries in a list of entries.
    ///
    /// A follower/learner reverts the effective membership to the previous one,
    /// when conflicting logs are found.
    fn last_two_memberships<'a>(entries: impl DoubleEndedIterator<Item = &'a Ent>) -> Vec<StoredMembership<NID, N>>
    where Ent: 'a {
        let mut memberships = vec![];

        // Find the last 2 membership config entries: the committed and the effective.
        for ent in entries.rev() {
            if let Some(m) = ent.get_membership() {
                memberships.insert(0, StoredMembership::new(Some(*ent.get_log_id()), m.clone()));
                if memberships.len() == 2 {
                    break;
                }
            }
        }

        memberships
    }

    fn log_handler(&mut self) -> LogHandler<NID, N, Ent> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }

    fn snapshot_handler(&mut self) -> SnapshotHandler<NID, N, Ent> {
        SnapshotHandler {
            state: self.state,
            output: self.output,
        }
    }

    fn server_state_handler(&mut self) -> ServerStateHandler<NID, N, Ent> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}
