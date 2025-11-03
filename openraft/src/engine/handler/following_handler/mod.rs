use std::sync::Arc;

use crate::EffectiveMembership;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::StoredMembership;
use crate::core::sm;
use crate::display_ext::DisplayOption;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySliceExt;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::entry::raft_entry_ext::RaftEntryExt;
use crate::error::RejectAppendEntries;
use crate::log_id::option_raft_log_id_ext::OptionRaftLogIdExt;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::storage::Snapshot;
use crate::type_config::alias::LogIdOf;
use crate::vote::committed::CommittedVote;
use crate::vote::raft_vote::RaftVoteExt;

#[cfg(test)]
mod append_entries_test;
#[cfg(test)]
mod do_append_entries_test;
#[cfg(test)]
mod install_snapshot_test;
#[cfg(test)]
mod truncate_logs_test;
#[cfg(test)]
mod update_committed_membership_test;

/// Receive replication request and deal with them.
///
/// It mainly implements the logic of a follower/learner
pub(crate) struct FollowingHandler<'x, C>
where C: RaftTypeConfig
{
    /// The Leader this Acceptor (Follower/Leaner) currently following.
    pub(crate) leader_vote: CommittedVote<C>,

    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

impl<C> FollowingHandler<'_, C>
where C: RaftTypeConfig
{
    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_entries(&mut self, prev_log_id: Option<LogIdOf<C>>, mut entries: Vec<C::Entry>) {
        tracing::debug!(
            "{}: local last_log_id: {}, request: prev_log_id: {}, entries: {}",
            func_name!(),
            self.state.last_log_id().display(),
            prev_log_id.display(),
            entries.display(),
        );

        if let Some(first_ent) = entries.first() {
            debug_assert!(first_ent.index() == prev_log_id.next_index());
        }

        let last_log_id = entries.last().map(|ent| ent.log_id());
        let last_log_id = std::cmp::max(prev_log_id, last_log_id);

        let prev_accepted = self.state.accept_log_io(IOId::new_log_io(self.leader_vote.clone(), last_log_id.clone()));

        let l = entries.len();
        let since = self.state.first_conflicting_index(&entries);
        if since < l {
            // Before appending, if an entry overrides a conflicting one,
            // the entries after it has to be deleted first.
            // Raft requires log ids are in total order by (term,index).
            // Otherwise, the log id with max index makes the committed entry invisible in election.
            self.truncate_logs(entries[since].index());

            let entries = entries.split_off(since);
            self.do_append_entries(entries);
        } else {
            // No actual IO is needed, but just need to update I/O state,
            // after all preceding IO flushed.

            let to_submit = IOId::new_log_io(self.leader_vote.clone(), last_log_id);

            if Some(&to_submit) <= prev_accepted.as_ref() {
                // No io state to update.
                return;
            }

            let condition = prev_accepted.map(|x| Condition::IOFlushed { io_id: x });

            self.output.push_command(Command::UpdateIOProgress {
                when: condition,
                io_id: to_submit,
            });
        }
    }

    /// Ensures the log to replicate is consecutive to the local log.
    ///
    /// If not, truncate the local log and return an error.
    pub(crate) fn ensure_log_consecutive(
        &mut self,
        prev_log_id: Option<&LogIdOf<C>>,
    ) -> Result<(), RejectAppendEntries<C>> {
        if let Some(prev) = prev_log_id
            && !self.state.has_log_id(prev)
        {
            let local = self.state.get_log_id(prev.index());
            tracing::debug!(local = display(DisplayOption(&local)), "prev_log_id does not match");

            self.truncate_logs(prev.index());
            return Err(RejectAppendEntries::ByConflictingLogId {
                local,
                expect: prev.clone(),
            });
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
    pub(crate) fn do_append_entries(&mut self, entries: Vec<C::Entry>) {
        debug_assert!(!entries.is_empty());
        debug_assert_eq!(entries[0].index(), self.state.log_ids.last().next_index(),);
        debug_assert!(Some(entries[0].ref_log_id()) > self.state.log_ids.last_ref());

        self.state.extend_log_ids(entries.iter().map(|ent| ent.ref_log_id()));
        self.append_membership(entries.iter());

        self.output.push_command(Command::AppendEntries {
            // A follower should always use the node's vote.
            committed_vote: self.leader_vote.clone(),
            entries,
        });
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
        self.output.push_command(Command::TruncateLog { since: since_log_id });

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
                last = display(&m),
                "applying {}-th new membership configs received from leader",
                i
            );
            self.state.membership_state.append(Arc::new(EffectiveMembership::new_from_stored_membership(m)));
        }

        tracing::debug!(
            membership_state = display(&self.state.membership_state),
            "updated membership state"
        );

        self.server_state_handler().update_server_state_if_changed();
    }

    /// Update membership state with a committed membership config
    #[tracing::instrument(level = "debug", skip_all)]
    fn update_committed_membership(&mut self, membership: EffectiveMembership<C>) {
        tracing::debug!("update committed membership: {}", membership);

        let m = Arc::new(membership);

        // TODO: if effective membership changes, call `update_replication()`, if a follower has replication
        //       streams. Now we don't have replication streams for follower, so it's ok to not call
        //       `update_replication()`.
        let _effective_changed = self.state.membership_state.update_committed(m);

        self.server_state_handler().update_server_state_if_changed();
    }

    /// Installs a full snapshot on a follower or learner node.
    ///
    /// Refer to [`snapshot_replication`](crate::docs::protocol::replication::snapshot_replication)
    /// for the reason the following workflow is needed.
    ///
    /// The method processes the given `snapshot` and updates the internal state of the node based
    /// on the snapshot's metadata. It checks if the `snapshot` is newer than the currently
    /// committed snapshot. If not, it does nothing.
    ///
    /// It returns the condition about when the snapshot is installed and can proceed with the
    /// commands that depend on the state of the snapshot.
    ///
    /// It returns an `Option<Condition<C>>` indicating the next action:
    /// - `Some(Condition::StateMachineCommand { command_seq })` if the snapshot will be installed.
    ///   Further commands that depend on the snapshot state should use this condition so that these
    ///   command block until the condition is satisfied(`RaftCore` receives a `Notification`).
    /// - Otherwise `None` if the snapshot will not be installed (e.g., if it is not newer than the
    ///   current state).
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn install_full_snapshot(&mut self, snapshot: Snapshot<C>) -> Option<Condition<C>> {
        let meta = &snapshot.meta;
        tracing::info!("install_full_snapshot: meta:{:?}", meta);

        let snap_last_log_id = meta.last_log_id.clone();

        if snap_last_log_id.as_ref() <= self.state.committed() {
            tracing::info!(
                "No need to install snapshot; snapshot last_log_id({}) <= committed({})",
                snap_last_log_id.display(),
                self.state.committed().display()
            );
            return None;
        }

        // snapshot_last_log_id cannot be None
        let snap_last_log_id = snap_last_log_id.unwrap();

        // 1. Truncate all logs if conflict.
        // 2. Install snapshot.
        // 3. Purge logs the snapshot covers.

        let mut snap_handler = self.snapshot_handler();
        let updated = snap_handler.update_snapshot(meta.clone());
        if !updated {
            return None;
        }

        let local = self.state.get_log_id(snap_last_log_id.index());
        if let Some(local) = local
            && local != snap_last_log_id
        {
            // Conflict, delete all non-committed logs.
            self.truncate_logs(self.state.committed().next_index());
        }

        let log_io_id = LogIOId::new(self.leader_vote.to_committed(), Some(snap_last_log_id.clone()));
        self.state.accept_log_io(log_io_id.to_io_id());

        self.state.apply_progress_mut().accept(snap_last_log_id.clone());
        self.state.snapshot_progress_mut().accept(snap_last_log_id.clone());

        self.update_committed_membership(EffectiveMembership::new_from_stored_membership(
            meta.last_membership.clone(),
        ));

        self.output.push_command(Command::from(sm::Command::install_full_snapshot(snapshot, log_io_id)));

        self.state.purge_upto = Some(snap_last_log_id.clone());
        self.log_handler().purge_log();

        Some(Condition::Snapshot {
            log_id: snap_last_log_id,
        })
    }

    /// Find the last 2 membership entries in a list of entries.
    ///
    /// A follower/learner reverts the effective membership to the previous one
    /// when conflicting logs are found.
    fn last_two_memberships<'a>(entries: impl DoubleEndedIterator<Item = &'a C::Entry>) -> Vec<StoredMembership<C>>
    where C::Entry: 'a {
        let mut memberships = vec![];

        // Find the last 2 membership config entries: the committed and the effective.
        for ent in entries.rev() {
            if let Some(m) = ent.get_membership() {
                memberships.insert(0, StoredMembership::new(Some(ent.log_id()), m));
                if memberships.len() == 2 {
                    break;
                }
            }
        }

        memberships
    }

    fn log_handler(&mut self) -> LogHandler<'_, C> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }

    fn snapshot_handler(&mut self) -> SnapshotHandler<'_, '_, C> {
        SnapshotHandler {
            state: self.state,
            output: self.output,
        }
    }

    fn server_state_handler(&mut self) -> ServerStateHandler<'_, C> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
        }
    }
}
