use std::sync::Arc;

use crate::core::sm;
use crate::display_ext::DisplayOption;
use crate::display_ext::DisplaySlice;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::entry::RaftPayload;
use crate::error::InstallSnapshotError;
use crate::error::RejectAppendEntries;
use crate::error::SnapshotMismatch;
use crate::raft::InstallSnapshotRequest;
use crate::raft_state::LogStateReader;
use crate::raft_state::StreamingState;
use crate::AsyncRuntime;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
use crate::RaftLogId;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::SnapshotSegmentId;
use crate::StoredMembership;

#[cfg(test)] mod append_entries_test;
#[cfg(test)] mod commit_entries_test;
#[cfg(test)] mod do_append_entries_test;
#[cfg(test)] mod install_snapshot_test;
#[cfg(test)] mod receive_snapshot_chunk_test;
#[cfg(test)] mod truncate_logs_test;
#[cfg(test)] mod update_committed_membership_test;

/// Receive replication request and deal with them.
///
/// It mainly implements the logic of a follower/learner
pub(crate) struct FollowingHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C::NodeId>,
    pub(crate) state: &'x mut RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

impl<'x, C> FollowingHandler<'x, C>
where C: RaftTypeConfig
{
    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_entries(&mut self, prev_log_id: Option<LogId<C::NodeId>>, entries: Vec<C::Entry>) {
        tracing::debug!(
            prev_log_id = display(prev_log_id.summary()),
            entries = display(DisplaySlice::<_>(&entries)),
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
    }

    /// Ensures the log to replicate is consecutive to the local log.
    ///
    /// If not, truncate the local log and return an error.
    pub(crate) fn ensure_log_consecutive(
        &mut self,
        prev_log_id: Option<LogId<C::NodeId>>,
    ) -> Result<(), RejectAppendEntries<C::NodeId>> {
        if let Some(ref prev) = prev_log_id {
            if !self.state.has_log_id(prev) {
                let local = self.state.get_log_id(prev.index);
                tracing::debug!(local = display(DisplayOption(&local)), "prev_log_id does not match");

                self.truncate_logs(prev.index);
                return Err(RejectAppendEntries::ByConflictingLogId { local, expect: *prev });
            }
        }

        // else `prev_log_id.is_none()` means replicating logs from the very beginning.

        Ok(())
    }

    /// Follower/Learner appends `entries[since..]`.
    ///
    /// It assumes:
    /// - Previous entries all match the leader's.
    /// - conflicting entries are deleted.
    ///
    /// Membership config changes are also detected and applied here.
    #[tracing::instrument(level = "debug", skip(self, entries))]
    fn do_append_entries(&mut self, mut entries: Vec<C::Entry>, since: usize) {
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

    /// Commit entries that are already committed by the leader.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn commit_entries(&mut self, leader_committed: Option<LogId<C::NodeId>>) {
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
            let seq = self.output.next_sm_seq();
            self.output.push_command(Command::Commit {
                seq,
                // TODO(xp): when restart, commit is reset to None. Use last_applied instead.
                already_committed: prev_committed,
                upto: committed.unwrap(),
            });

            if self.config.snapshot_policy.should_snapshot(&self.state) {
                self.snapshot_handler().trigger_snapshot();
            }
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
        if let Some(_c) = changed {
            self.server_state_handler().update_server_state_if_changed();
        }
    }

    /// Append membership log if membership config entries are found, after appending entries to
    /// log.
    fn append_membership<'a>(&mut self, entries: impl DoubleEndedIterator<Item = &'a C::Entry>)
    where C::Entry: 'a {
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

        self.server_state_handler().update_server_state_if_changed();
    }

    /// Update membership state with a committed membership config
    #[tracing::instrument(level = "debug", skip_all)]
    fn update_committed_membership(&mut self, membership: EffectiveMembership<C::NodeId, C::Node>) {
        tracing::debug!("update committed membership: {}", membership.summary());

        let m = Arc::new(membership);

        // TODO: if effective membership changes, call `update_replication()`, if a follower has replication
        //       streams. Now we don't have replication streams for follower, so it's ok to not call
        //       `update_replication()`.
        let _effective_changed = self.state.membership_state.update_committed(m);

        self.server_state_handler().update_server_state_if_changed();
    }

    /// receives a snapshot chunk.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn receive_snapshot_chunk(
        &mut self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<(), InstallSnapshotError> {
        tracing::info!(req = display(req.summary()), "{}", func_name!());

        let snapshot_id = &req.meta.snapshot_id;

        let curr_id = self.state.snapshot_streaming.as_ref().map(|s| &s.snapshot_id);

        // Changed to another stream. re-init snapshot state.
        if curr_id != Some(&req.meta.snapshot_id) {
            if req.offset > 0 {
                let mismatch = SnapshotMismatch {
                    expect: SnapshotSegmentId {
                        id: snapshot_id.clone(),
                        offset: 0,
                    },
                    got: SnapshotSegmentId {
                        id: snapshot_id.clone(),
                        offset: req.offset,
                    },
                };

                tracing::warn!("snapshot mismatch: {}", mismatch);
                return Err(mismatch.into());
            }

            if req.offset == 0 {
                self.state.snapshot_streaming = Some(StreamingState {
                    offset: 0,
                    snapshot_id: snapshot_id.clone(),
                });
            }
        }

        self.state.snapshot_streaming.as_mut().unwrap().offset = req.offset;

        let sm_cmd = sm::Command::receive(req);
        self.output.push_command(Command::from(sm_cmd));
        Ok(())
    }

    /// Follower/Learner handles install-snapshot.
    ///
    /// Refer to [`snapshot_replication`](crate::docs::protocol::replication::snapshot_replication)
    /// for the reason the following workflow is needed.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn install_snapshot(&mut self, meta: SnapshotMeta<C::NodeId, C::Node>) {
        tracing::info!("install_snapshot: meta:{:?}", meta);

        let snap_last_log_id = meta.last_log_id;

        if snap_last_log_id.as_ref() <= self.state.committed() {
            tracing::info!(
                "No need to install snapshot; snapshot last_log_id({}) <= committed({})",
                snap_last_log_id.summary(),
                self.state.committed().summary()
            );
            self.output.push_command(Command::from(sm::Command::cancel_snapshot(meta)));
            return;
        }

        // snapshot_last_log_id can not be None
        let snap_last_log_id = snap_last_log_id.unwrap();

        // 1. Truncate all logs if conflict.
        // 2. Install snapshot.
        // 3. Purge logs the snapshot covers.

        let mut snap_handler = self.snapshot_handler();
        let updated = snap_handler.update_snapshot(meta.clone());
        if !updated {
            return;
        }

        let local = self.state.get_log_id(snap_last_log_id.index);
        if let Some(local) = local {
            if local != snap_last_log_id {
                // Conflict, delete all non-committed logs.
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
        self.output.push_command(Command::from(sm::Command::install_snapshot(meta)));

        // TODO: move this to LogHandler::purge_log()?
        self.state.purge_upto = Some(snap_last_log_id);
        self.log_handler().purge_log();
    }

    /// Find the last 2 membership entries in a list of entries.
    ///
    /// A follower/learner reverts the effective membership to the previous one,
    /// when conflicting logs are found.
    fn last_two_memberships<'a>(
        entries: impl DoubleEndedIterator<Item = &'a C::Entry>,
    ) -> Vec<StoredMembership<C::NodeId, C::Node>>
    where C::Entry: 'a {
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

    fn log_handler(&mut self) -> LogHandler<C> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }

    fn snapshot_handler(&mut self) -> SnapshotHandler<C> {
        SnapshotHandler {
            state: self.state,
            output: self.output,
        }
    }

    fn server_state_handler(&mut self) -> ServerStateHandler<C> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}
