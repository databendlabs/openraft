use std::sync::Arc;

use crate::engine::engine_impl::EngineOutput;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::entry::RaftEntry;
use crate::raft_state::LogStateReader;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::SnapshotMeta;

// TODO: move tests to this file
/// Receive replication request and deal with them.
///
/// It mainly implements the logic of a follower/learner
pub(crate) struct FollowingHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: &'x mut EngineConfig<NID>,
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
}

impl<'x, NID, N> FollowingHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Append membership log if membership config entries are found, after appending entries to
    /// log.
    fn follower_append_membership<'a, Ent: RaftEntry<NID, N> + 'a>(
        &mut self,
        entries: impl DoubleEndedIterator<Item = &'a Ent>,
    ) {
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
            self.state.membership_state.append(Arc::new(m));
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

        // TODO(5): follower has not yet commit the membership_state.
        //          For now it is OK. But it should be done here.
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

        self.follower_append_membership(entries.iter());

        // TODO(xp): should be moved to handle_append_entries_req()
        self.output.push_command(Command::MoveInputCursorBy { n: l });
    }

    /// Delete log entries since log index `since`, inclusive, when the log at `since` is found
    /// conflict with the leader.
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

        let changed = self.state.membership_state.truncate(since);
        if let Some(c) = changed {
            self.output.push_command(Command::UpdateMembership { membership: c });
            self.server_state_handler().update_server_state_if_changed();
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

        self.state.committed = Some(snap_last_log_id);
        self.update_committed_membership(meta.last_membership.clone());

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

    pub(crate) fn log_handler(&mut self) -> LogHandler<NID, N> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }

    pub(crate) fn snapshot_handler(&mut self) -> SnapshotHandler<NID, N> {
        SnapshotHandler {
            state: self.state,
            output: self.output,
        }
    }

    fn server_state_handler(&mut self) -> ServerStateHandler<NID, N> {
        ServerStateHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}

#[cfg(test)]
mod tests {
    mod follower_commit_entries_test {

        use std::sync::Arc;

        use maplit::btreeset;

        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::raft_state::LogStateReader;
        use crate::EffectiveMembership;
        use crate::Entry;
        use crate::EntryPayload;
        use crate::LeaderId;
        use crate::LogId;
        use crate::Membership;
        use crate::MembershipState;
        use crate::MetricsChangeFlags;

        crate::declare_raft_types!(
            pub(crate) Foo: D=(), R=(), NodeId=u64, Node = ()
        );

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

        fn blank(term: u64, index: u64) -> Entry<Foo> {
            Entry {
                log_id: log_id(term, index),
                payload: EntryPayload::<Foo>::Blank,
            }
        }

        fn m01() -> Membership<u64, ()> {
            Membership::new(vec![btreeset! {0,1}], None)
        }

        fn m23() -> Membership<u64, ()> {
            Membership::new(vec![btreeset! {2,3}], None)
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.state.committed = Some(log_id(1, 1));
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            );
            eng
        }

        #[test]
        fn test_follower_commit_entries_empty() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().follower_commit_entries(None, None, &Vec::<Entry<Foo>>::new());

            assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                ),
                eng.state.membership_state
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(0, eng.output.commands.len());

            Ok(())
        }

        #[test]
        fn test_follower_commit_entries_no_update() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().follower_commit_entries(Some(log_id(1, 1)), None, &[blank(2, 4)]);

            assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                ),
                eng.state.membership_state
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(0, eng.output.commands.len());

            Ok(())
        }

        #[test]
        fn test_follower_commit_entries_lt_last_entry() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().follower_commit_entries(Some(log_id(2, 3)), None, &[blank(2, 3)]);

            assert_eq!(Some(&log_id(2, 3)), eng.state.committed());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                ),
                eng.state.membership_state
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::FollowerCommit {
                    already_committed: Some(log_id(1, 1)),
                    upto: log_id(2, 3)
                }],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_follower_commit_entries_gt_last_entry() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().follower_commit_entries(Some(log_id(3, 1)), None, &[blank(2, 3)]);

            assert_eq!(Some(&log_id(2, 3)), eng.state.committed());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
                ),
                eng.state.membership_state
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::FollowerCommit {
                    already_committed: Some(log_id(1, 1)),
                    upto: log_id(2, 3)
                }],
                eng.output.commands
            );

            Ok(())
        }
    }

    mod follower_do_append_entries_test {

        use std::sync::Arc;

        use maplit::btreeset;

        use crate::core::ServerState;
        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::raft_state::LogStateReader;
        use crate::EffectiveMembership;
        use crate::Entry;
        use crate::EntryPayload;
        use crate::LeaderId;
        use crate::LogId;
        use crate::Membership;
        use crate::MembershipState;
        use crate::MetricsChangeFlags;

        crate::declare_raft_types!(
            pub(crate) Foo: D=(), R=(), NodeId=u64, Node = ()
        );

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

        fn blank(term: u64, index: u64) -> Entry<Foo> {
            Entry {
                log_id: log_id(term, index),
                payload: EntryPayload::<Foo>::Blank,
            }
        }

        fn m01() -> Membership<u64, ()> {
            Membership::new(vec![btreeset! {0,1}], None)
        }

        fn m23() -> Membership<u64, ()> {
            Membership::new(vec![btreeset! {2,3}], None)
        }

        fn m34() -> Membership<u64, ()> {
            Membership::new(vec![btreeset! {3,4}], None)
        }

        fn m45() -> Membership<u64, ()> {
            Membership::new(vec![btreeset! {4,5}], None)
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.config.id = 2;
            eng.state.log_ids.append(log_id(1, 1));
            eng.state.log_ids.append(log_id(2, 3));
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            );
            eng.state.server_state = eng.calc_server_state();
            eng
        }

        #[test]
        fn test_follower_do_append_entries_empty() -> anyhow::Result<()> {
            let mut eng = eng();

            // Neither of these two will update anything.
            eng.following_handler().follower_do_append_entries(&Vec::<Entry<Foo>>::new(), 0);
            eng.following_handler().follower_do_append_entries(&[blank(3, 4)], 1);

            assert_eq!(
                &[
                    log_id(1, 1), //
                    log_id(2, 3),
                ],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                ),
                eng.state.membership_state
            );
            assert_eq!(ServerState::Follower, eng.state.server_state);

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(0, eng.output.commands.len());

            Ok(())
        }

        #[test]
        fn test_follower_do_append_entries_no_membership_entries() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().follower_do_append_entries(
                &[
                    blank(100, 100), // just be ignored
                    blank(3, 4),
                ],
                1,
            );

            assert_eq!(
                &[
                    log_id(1, 1), //
                    log_id(2, 3),
                    log_id(3, 4),
                ],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(Some(&log_id(3, 4)), eng.state.last_log_id());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                ),
                eng.state.membership_state
            );
            assert_eq!(ServerState::Follower, eng.state.server_state);

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    Command::AppendInputEntries { range: 1..2 },
                    Command::MoveInputCursorBy { n: 2 }
                ],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_follower_do_append_entries_one_membership_entry() -> anyhow::Result<()> {
            // - The membership entry in the input becomes effective membership. The previous effective becomes
            //   committed.
            // - Follower become Learner, since it is not in the new effective membership.
            let mut eng = eng();
            eng.config.id = 2; // make it a member, the become learner

            eng.following_handler().follower_do_append_entries(
                &[
                    blank(3, 3), // ignored
                    blank(3, 3), // ignored
                    blank(3, 3), // ignored
                    blank(3, 4),
                    Entry {
                        log_id: log_id(3, 5),
                        payload: EntryPayload::<Foo>::Membership(m34()),
                    },
                ],
                3,
            );

            assert_eq!(
                &[
                    log_id(1, 1), //
                    log_id(2, 3),
                    log_id(3, 4),
                    log_id(3, 5),
                ],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(Some(&log_id(3, 5)), eng.state.last_log_id());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                    Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
                ),
                eng.state.membership_state,
                "previous effective become committed"
            );
            assert_eq!(
                ServerState::Learner,
                eng.state.server_state,
                "not in membership, become learner"
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    Command::AppendInputEntries { range: 3..5 },
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
                    },
                    Command::MoveInputCursorBy { n: 5 }
                ],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_follower_do_append_entries_three_membership_entries() -> anyhow::Result<()> {
            // - The last 2 of the 3 membership entries take effect.
            // - A learner become follower.

            let mut eng = eng();
            eng.config.id = 5; // make it a learner, then become follower
            eng.state.server_state = eng.calc_server_state();

            eng.following_handler().follower_do_append_entries(
                &[
                    Entry {
                        log_id: log_id(3, 4),
                        payload: EntryPayload::<Foo>::Membership(m01()),
                    }, // ignored
                    blank(3, 4),
                    Entry {
                        log_id: log_id(3, 5),
                        payload: EntryPayload::<Foo>::Membership(m01()),
                    },
                    Entry {
                        log_id: log_id(4, 6),
                        payload: EntryPayload::<Foo>::Membership(m34()),
                    },
                    Entry {
                        log_id: log_id(4, 7),
                        payload: EntryPayload::<Foo>::Membership(m45()),
                    },
                ],
                1,
            );

            assert_eq!(
                &[
                    log_id(1, 1), //
                    log_id(2, 3),
                    log_id(3, 4),
                    log_id(4, 6),
                    log_id(4, 7),
                ],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(Some(&log_id(4, 7)), eng.state.last_log_id());
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(4, 6)), m34())),
                    Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45())),
                ),
                eng.state.membership_state,
                "seen 3 membership, the last 2 become committed and effective"
            );
            assert_eq!(
                ServerState::Follower,
                eng.state.server_state,
                "in membership, become follower"
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    Command::AppendInputEntries { range: 1..5 },
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45())),
                    },
                    Command::MoveInputCursorBy { n: 5 }
                ],
                eng.output.commands
            );

            Ok(())
        }
    }

    mod truncate_logs_test {

        use std::sync::Arc;

        use maplit::btreeset;

        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::engine::LogIdList;
        use crate::raft_state::LogStateReader;
        use crate::EffectiveMembership;
        use crate::LeaderId;
        use crate::LogId;
        use crate::Membership;
        use crate::MembershipState;
        use crate::MetricsChangeFlags;
        use crate::ServerState;

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

        fn m01() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
        }

        fn m12() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
        }

        fn m23() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.config.id = 2;
            eng.state.log_ids = LogIdList::new(vec![
                log_id(2, 2), //
                log_id(4, 4),
                log_id(4, 6),
            ]);
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            );

            eng.state.server_state = ServerState::Follower;
            eng
        }

        #[test]
        fn test_truncate_logs_since_3() -> anyhow::Result<()> {
            let mut eng = eng();
            eng.state.server_state = eng.calc_server_state();

            eng.following_handler().truncate_logs(3);

            assert_eq!(Some(&log_id(2, 2)), eng.state.last_log_id());
            assert_eq!(&[log_id(2, 2)], eng.state.log_ids.key_log_ids());
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                ),
                eng.state.membership_state
            );
            assert_eq!(ServerState::Learner, eng.state.server_state);

            assert_eq!(
                vec![
                    //
                    Command::DeleteConflictLog { since: log_id(2, 3) },
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
                    },
                ],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_truncate_logs_since_4() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().truncate_logs(4);

            assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
            assert_eq!(&[log_id(2, 2), log_id(2, 3)], eng.state.log_ids.key_log_ids());
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: false,
                },
                eng.output.metrics_flags
            );
            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                ),
                eng.state.membership_state
            );
            assert_eq!(ServerState::Follower, eng.state.server_state);

            assert_eq!(
                vec![Command::DeleteConflictLog { since: log_id(4, 4) }],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_truncate_logs_since_5() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().truncate_logs(5);

            assert_eq!(Some(&log_id(4, 4)), eng.state.last_log_id());
            assert_eq!(&[log_id(2, 2), log_id(4, 4)], eng.state.log_ids.key_log_ids());
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::DeleteConflictLog { since: log_id(4, 5) }],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_truncate_logs_since_6() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().truncate_logs(6);

            assert_eq!(Some(&log_id(4, 5)), eng.state.last_log_id());
            assert_eq!(
                &[log_id(2, 2), log_id(4, 4), log_id(4, 5)],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::DeleteConflictLog { since: log_id(4, 6) }],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_truncate_logs_since_7() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().truncate_logs(7);

            assert_eq!(Some(&log_id(4, 6)), eng.state.last_log_id());
            assert_eq!(
                &[log_id(2, 2), log_id(4, 4), log_id(4, 6)],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert!(eng.output.commands.is_empty());

            Ok(())
        }

        #[test]
        fn test_truncate_logs_since_8() -> anyhow::Result<()> {
            let mut eng = eng();

            eng.following_handler().truncate_logs(8);

            assert_eq!(Some(&log_id(4, 6)), eng.state.last_log_id());
            assert_eq!(
                &[log_id(2, 2), log_id(4, 4), log_id(4, 6)],
                eng.state.log_ids.key_log_ids()
            );
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert!(eng.output.commands.is_empty());

            Ok(())
        }

        #[test]
        fn test_truncate_logs_revert_effective_membership() -> anyhow::Result<()> {
            let mut eng = eng();
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(4, 4)), m12())),
            );
            eng.state.server_state = eng.calc_server_state();

            eng.following_handler().truncate_logs(4);

            assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
            assert_eq!(&[log_id(2, 2), log_id(2, 3)], eng.state.log_ids.key_log_ids());
            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    //
                    Command::DeleteConflictLog { since: log_id(4, 4) },
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m01()))
                    },
                ],
                eng.output.commands
            );

            Ok(())
        }
    }

    mod update_committed_membership_test {

        use std::sync::Arc;

        use maplit::btreeset;

        use crate::core::ServerState;
        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::EffectiveMembership;
        use crate::LeaderId;
        use crate::LogId;
        use crate::Membership;
        use crate::MembershipState;
        use crate::MetricsChangeFlags;

        crate::declare_raft_types!(
            pub(crate) Foo: D=(), R=(), NodeId=u64, Node=()
        );

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

        fn m01() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
        }

        fn m23() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
        }

        fn m34() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {3,4}], None)
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.config.id = 2;
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            );

            eng.state.server_state = eng.calc_server_state();
            eng
        }

        #[test]
        fn test_update_committed_membership_at_index_4() -> anyhow::Result<()> {
            // replace effective membership
            let mut eng = eng();

            eng.following_handler()
                .update_committed_membership(EffectiveMembership::new(Some(log_id(3, 4)), m34()));

            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34())),
                    Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34()))
                ),
                eng.state.membership_state
            );
            assert_eq!(ServerState::Learner, eng.state.server_state);

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::UpdateMembership {
                    membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34())),
                },],
                eng.output.commands
            );

            Ok(())
        }
    }

    mod install_snapshot_test {

        use std::sync::Arc;

        use maplit::btreeset;
        use pretty_assertions::assert_eq;

        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::engine::LogIdList;
        use crate::raft_state::LogStateReader;
        use crate::EffectiveMembership;
        use crate::LeaderId;
        use crate::LogId;
        use crate::Membership;
        use crate::MetricsChangeFlags;
        use crate::SnapshotMeta;

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

        fn m12() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
        }

        fn m1234() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::<u64, ()> { ..Default::default() };
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.state.committed = Some(log_id(4, 5));
            eng.state.log_ids = LogIdList::new(vec![
                //
                log_id(2, 2),
                log_id(3, 5),
                log_id(4, 6),
                log_id(4, 8),
            ]);
            eng.state.snapshot_meta = SnapshotMeta {
                last_log_id: Some(log_id(2, 2)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
                snapshot_id: "1-2-3-4".to_string(),
            };
            eng.state.server_state = eng.calc_server_state();

            eng
        }

        #[test]
        fn test_install_snapshot_lt_last_snapshot() -> anyhow::Result<()> {
            // Snapshot will not be installed because new `last_log_id` is less or equal current
            // `snapshot_meta.last_log_id`.
            let mut eng = eng();

            eng.following_handler().install_snapshot(SnapshotMeta {
                last_log_id: Some(log_id(2, 2)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            });

            assert_eq!(
                SnapshotMeta {
                    last_log_id: Some(log_id(2, 2)),
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
                    snapshot_id: "1-2-3-4".to_string(),
                },
                eng.state.snapshot_meta
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::CancelSnapshot {
                    snapshot_meta: SnapshotMeta {
                        last_log_id: Some(log_id(2, 2)),
                        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                        snapshot_id: "1-2-3-4".to_string(),
                    }
                }],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_install_snapshot_lt_committed() -> anyhow::Result<()> {
            // Snapshot will not be installed because new `last_log_id` is less or equal current
            // `committed`. TODO: The snapshot should be able to be updated if
            // `new_snapshot.last_log_id > engine.snapshot_meta.last_log_id`.
            // Although in this case the state machine is not affected.
            let mut eng = eng();

            eng.following_handler().install_snapshot(SnapshotMeta {
                last_log_id: Some(log_id(4, 5)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            });

            assert_eq!(
                SnapshotMeta {
                    last_log_id: Some(log_id(2, 2)),
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
                    snapshot_id: "1-2-3-4".to_string(),
                },
                eng.state.snapshot_meta
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: false,
                    cluster: false,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![Command::CancelSnapshot {
                    snapshot_meta: SnapshotMeta {
                        last_log_id: Some(log_id(4, 5)),
                        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                        snapshot_id: "1-2-3-4".to_string(),
                    }
                }],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_install_snapshot_not_conflict() -> anyhow::Result<()> {
            // Snapshot will be installed and there are no conflicting logs.
            let mut eng = eng();

            eng.following_handler().install_snapshot(SnapshotMeta {
                last_log_id: Some(log_id(4, 6)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            });

            assert_eq!(
                SnapshotMeta {
                    last_log_id: Some(log_id(4, 6)),
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                },
                eng.state.snapshot_meta
            );
            assert_eq!(&[log_id(4, 6), log_id(4, 8)], eng.state.log_ids.key_log_ids());
            assert_eq!(Some(&log_id(4, 6)), eng.state.committed());
            assert_eq!(
                &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
                eng.state.membership_state.committed()
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()))
                    },
                    //
                    Command::InstallSnapshot {
                        snapshot_meta: SnapshotMeta {
                            last_log_id: Some(log_id(4, 6)),
                            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                            snapshot_id: "1-2-3-4".to_string(),
                        }
                    },
                    Command::PurgeLog { upto: log_id(4, 6) },
                ],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_install_snapshot_conflict() -> anyhow::Result<()> {
            // Snapshot will be installed, all non-committed log will be deleted.
            // And there should be no conflicting logs left.
            let mut eng = {
                let mut eng = Engine::<u64, ()> { ..Default::default() };
                eng.state.enable_validate = false; // Disable validation for incomplete state

                eng.state.committed = Some(log_id(2, 3));
                eng.state.log_ids = LogIdList::new(vec![
                    //
                    log_id(2, 2),
                    log_id(3, 5),
                    log_id(4, 6),
                    log_id(4, 8),
                ]);

                eng.state.snapshot_meta = SnapshotMeta {
                    last_log_id: Some(log_id(2, 2)),
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
                    snapshot_id: "1-2-3-4".to_string(),
                };

                eng.state.server_state = eng.calc_server_state();

                eng
            };

            eng.following_handler().install_snapshot(SnapshotMeta {
                last_log_id: Some(log_id(5, 6)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            });

            assert_eq!(
                SnapshotMeta {
                    last_log_id: Some(log_id(5, 6)),
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                },
                eng.state.snapshot_meta
            );
            assert_eq!(&[log_id(5, 6)], eng.state.log_ids.key_log_ids());
            assert_eq!(Some(&log_id(5, 6)), eng.state.committed());
            assert_eq!(
                &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
                eng.state.membership_state.committed()
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    Command::DeleteConflictLog { since: log_id(2, 4) },
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()))
                    },
                    //
                    Command::InstallSnapshot {
                        snapshot_meta: SnapshotMeta {
                            last_log_id: Some(log_id(5, 6)),
                            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                            snapshot_id: "1-2-3-4".to_string(),
                        }
                    },
                    Command::PurgeLog { upto: log_id(5, 6) },
                ],
                eng.output.commands
            );

            Ok(())
        }

        #[test]
        fn test_install_snapshot_advance_last_log_id() -> anyhow::Result<()> {
            // Snapshot will be installed and there are no conflicting logs.
            let mut eng = eng();

            eng.following_handler().install_snapshot(SnapshotMeta {
                last_log_id: Some(log_id(100, 100)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            });

            assert_eq!(
                SnapshotMeta {
                    last_log_id: Some(log_id(100, 100)),
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                },
                eng.state.snapshot_meta
            );
            assert_eq!(&[log_id(100, 100)], eng.state.log_ids.key_log_ids());
            assert_eq!(Some(&log_id(100, 100)), eng.state.committed());
            assert_eq!(
                &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
                eng.state.membership_state.committed()
            );
            assert_eq!(
                &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
                eng.state.membership_state.effective()
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: false,
                    local_data: true,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()))
                    },
                    //
                    Command::InstallSnapshot {
                        snapshot_meta: SnapshotMeta {
                            last_log_id: Some(log_id(100, 100)),
                            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                            snapshot_id: "1-2-3-4".to_string(),
                        }
                    },
                    Command::PurgeLog { upto: log_id(100, 100) },
                ],
                eng.output.commands
            );

            Ok(())
        }
    }
}
