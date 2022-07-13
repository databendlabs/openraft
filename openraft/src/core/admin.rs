use std::collections::BTreeSet;
use std::mem::swap;
use std::option::Option::None;

use tracing::Level;

use crate::core::replication_state::replication_lag;
use crate::core::Expectation;
use crate::core::LeaderState;
use crate::entry::EntryRef;
use crate::error::AddLearnerError;
use crate::error::ChangeMembershipError;
use crate::error::ClientWriteError;
use crate::error::EmptyMembership;
use crate::error::Fatal;
use crate::error::InProgress;
use crate::error::LearnerIsLagging;
use crate::error::LearnerNotFound;
use crate::metrics::RemoveTarget;
use crate::progress::Progress;
use crate::raft::AddLearnerResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::RaftRespTx;
use crate::raft_types::RaftLogId;
use crate::runtime::RaftRuntime;
use crate::summary::MessageSummary;
use crate::versioned::Updatable;
use crate::ChangeMembers;
use crate::EntryPayload;
use crate::LogId;
use crate::Node;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    /// Add a new node to the cluster as a learner, bringing it up-to-speed, and then responding
    /// on the given channel.
    ///
    /// Adding a learner does not affect election, thus it does not need to enter joint consensus.
    ///
    /// And it does not need to wait for the previous membership log to commit to propose the new membership log.
    ///
    /// If `blocking` is `true`, the result is sent to `tx` as the target node log has caught up. Otherwise, result is
    /// sent at once, no matter whether the target node log is lagging or not.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn add_learner(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
        tx: RaftRespTx<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>>,
    ) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!(
            "add target node {} as learner; current nodes: {:?}",
            target,
            self.nodes.keys()
        );

        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes already being synced, or in the nodes being removed.

        let curr = &self.core.engine.state.membership_state.effective;
        if curr.contains(&target) {
            let matched = if let Some(l) = &self.core.engine.state.leader {
                *l.progress.get(&target)
            } else {
                unreachable!("it has to be a leader!!!");
            };

            tracing::debug!(
                "target {:?} already member or learner, can't add; matched:{:?}",
                target,
                matched
            );

            let _ = tx.send(Ok(AddLearnerResponse {
                membership_log_id: self.core.engine.state.membership_state.effective.log_id,
                matched,
            }));
            return Ok(());
        }

        let curr = &self.core.engine.state.membership_state.effective.membership;
        let res = curr.add_learner(target, node);
        let new_membership = match res {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(Err(AddLearnerError::MissingNodeInfo(e)));
                return Ok(());
            }
        };

        tracing::debug!(?new_membership, "new_membership with added learner: {}", target);

        let log_id = self.write_entry(EntryPayload::Membership(new_membership), None).await?;

        tracing::debug!(
            "after add target node {} as learner {:?}",
            target,
            self.core.engine.state.last_log_id()
        );

        let _ = tx.send(Ok(AddLearnerResponse {
            membership_log_id: Some(log_id),
            matched: None,
        }));

        Ok(())
    }

    /// return true if there is pending uncommitted config change
    fn has_pending_config(&self) -> bool {
        // The last membership config is not committed yet.
        // Can not process the next one.
        self.core.engine.state.committed < self.core.engine.state.membership_state.effective.log_id
    }

    /// Submit change-membership by writing a Membership log entry, if the `expect` is satisfied.
    ///
    /// If `turn_to_learner` is `true`, removed `voter` will becomes `learner`. Otherwise they will be just removed.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        changes: ChangeMembers<C::NodeId>,
        expectation: Option<Expectation>,
        turn_to_learner: bool,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>,
    ) -> Result<(), Fatal<C::NodeId>> {
        let last = self.core.engine.state.membership_state.effective.membership.get_joint_config().last().unwrap();
        let members = changes.apply_to(last);

        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::EmptyMembership(EmptyMembership {}),
            )));
            return Ok(());
        }

        if self.has_pending_config() {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::InProgress(InProgress {
                    // has_pending_config() implies an existing membership log.
                    membership_log_id: self.core.engine.state.membership_state.effective.log_id.unwrap(),
                }),
            )));
            return Ok(());
        }

        let mem = &self.core.engine.state.membership_state.effective;
        let curr = mem.membership.clone();

        let old_members = mem.voter_ids().collect::<BTreeSet<_>>();
        let only_in_new = members.difference(&old_members);

        let new_config = {
            let res = curr.next_safe(members.clone(), turn_to_learner);
            match res {
                Ok(x) => x,
                Err(e) => {
                    let change_err = ChangeMembershipError::MissingNodeInfo(e);
                    let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(change_err)));
                    return Ok(());
                }
            }
        };

        tracing::debug!(?new_config, "new_config");

        for node_id in only_in_new.clone() {
            if !mem.contains(node_id) {
                let not_found = LearnerNotFound { node_id: *node_id };
                let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                    ChangeMembershipError::LearnerNotFound(not_found),
                )));
                return Ok(());
            }
        }

        if let Err(e) = self.check_replication_states(only_in_new, expectation) {
            let _ = tx.send(Err(e.into()));
            return Ok(());
        }

        self.write_entry(EntryPayload::Membership(new_config), Some(tx)).await?;
        Ok(())
    }

    /// return Ok if all the current replication states satisfy the `expectation` for changing membership.
    fn check_replication_states<'n>(
        &self,
        nodes: impl Iterator<Item = &'n C::NodeId>,
        expectation: Option<Expectation>,
    ) -> Result<(), ChangeMembershipError<C::NodeId>> {
        let expectation = match &expectation {
            None => {
                // No expectation, whatever is OK.
                return Ok(());
            }
            Some(x) => x,
        };

        let last_log_id = self.core.engine.state.last_log_id();

        for node_id in nodes {
            match expectation {
                Expectation::AtLineRate => {
                    // Expect to be at line rate but not.

                    let matched = if let Some(l) = &self.core.engine.state.leader {
                        *l.progress.get(node_id)
                    } else {
                        unreachable!("it has to be a leader!!!");
                    };

                    let distance = replication_lag(&matched, &last_log_id);

                    if distance <= self.core.config.replication_lag_threshold {
                        continue;
                    }

                    let lagging = LearnerIsLagging {
                        node_id: *node_id,
                        matched,
                        distance,
                    };

                    return Err(ChangeMembershipError::LearnerIsLagging(lagging));
                }
            }
        }

        Ok(())
    }

    /// Write a log entry to the cluster through raft protocol.
    ///
    /// I.e.: append the log entry to local store, forward it to a quorum(including the leader), waiting for it to be
    /// committed and applied.
    ///
    /// The result of applying it to state machine is sent to `resp_tx`, if it is not `None`.
    /// The calling side may not receive a result from `resp_tx`, if raft is shut down.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(self.core.id)))]
    pub async fn write_entry(
        &mut self,
        payload: EntryPayload<C>,
        resp_tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,
    ) -> Result<LogId<C::NodeId>, Fatal<C::NodeId>> {
        tracing::debug!(payload = display(payload.summary()), "write_entry");

        let mut entry_refs = [EntryRef::new(&payload)];
        // TODO: it should returns membership config error etc. currently this is done by the caller.
        self.core.engine.leader_append_entries(&mut entry_refs);

        // Install callback channels.
        if let Some(tx) = resp_tx {
            if let Some(l) = &mut self.core.leader_data {
                l.client_resp_channels.insert(entry_refs[0].log_id.index, tx);
            }
        }

        self.run_engine_commands(&entry_refs).await?;

        Ok(*entry_refs[0].get_log_id())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn run_engine_commands<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
    ) -> Result<(), StorageError<C::NodeId>> {
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!("LeaderState run command: start...");
            for c in self.core.engine.commands.iter() {
                tracing::debug!("LeaderState run command: {:?}", c);
            }
        }

        let mut curr = 0;
        let mut commands = vec![];
        swap(&mut self.core.engine.commands, &mut commands);
        for cmd in commands.iter() {
            self.run_command(input_entries, &mut curr, cmd).await?;
        }

        Ok(())
    }

    /// Remove a replication if the membership that does not include it has committed.
    ///
    /// Return true if removed.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn remove_replication(&mut self, target: C::NodeId) -> bool {
        tracing::info!("removed_replication to: {}", target);

        let repl_state = self.nodes.remove(&target);
        if let Some(s) = repl_state {
            let handle = s.handle;

            // Drop sender to notify the task to shutdown
            drop(s.repl_tx);

            tracing::debug!("joining removed replication: {}", target);
            let _x = handle.await;
            tracing::info!("Done joining removed replication : {}", target);
        } else {
            unreachable!("try to nonexistent replication to {}", target);
        }

        if let Some(l) = &mut self.core.leader_data {
            l.replication_metrics.update(RemoveTarget { target });
        } else {
            unreachable!("It has to be a leader!!!");
        }

        // TODO(xp): set_replication_metrics_changed() can be removed.
        //           Use self.replication_metrics.version to detect changes.
        self.core.engine.metrics_flags.set_replication_changed();

        true
    }
}
