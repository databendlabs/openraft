use std::collections::BTreeSet;
use std::mem::swap;
use std::option::Option::None;

use tracing::Level;

use crate::config::RemoveReplicationPolicy;
use crate::core::replication_state::ReplicationState;
use crate::core::Expectation;
use crate::core::LeaderState;
use crate::core::ServerState;
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
use crate::raft::AddLearnerResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::RaftRespTx;
use crate::raft_types::LogIdOptionExt;
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
    // add node into learner,return true if the node is already a member or learner
    #[tracing::instrument(level = "debug", skip(self))]
    async fn write_add_learner_entry(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
    ) -> Result<LogId<C::NodeId>, AddLearnerError<C::NodeId>> {
        let curr = &self.core.engine.state.membership_state.effective.membership;
        let new_membership = curr.add_learner(target, node)?;

        tracing::debug!(?new_membership, "new_config");

        let log_id = self.write_entry(EntryPayload::Membership(new_membership), None).await?;

        Ok(log_id)
    }

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
        // TODO: remove this
        if target == self.core.id {
            tracing::debug!("target node is this node");

            let _ = tx.send(Ok(AddLearnerResponse {
                membership_log_id: self.core.engine.state.membership_state.effective.log_id,
                matched: self.core.engine.state.last_log_id(),
            }));
            return Ok(());
        }

        let curr = &self.core.engine.state.membership_state.effective;
        if curr.contains(&target) {
            tracing::debug!("target {:?} already member or learner, can't add", target);

            if let Some(t) = self.nodes.get(&target) {
                tracing::debug!("target node is already a cluster member or is being synced");
                let _ = tx.send(Ok(AddLearnerResponse {
                    membership_log_id: self.core.engine.state.membership_state.effective.log_id,
                    matched: t.matched,
                }));
                return Ok(());
            } else {
                unreachable!(
                    "node {} in membership but there is no replication stream for it",
                    target
                )
            }
        }

        // TODO(xp): when new membership log is appended, write_entry() should be responsible to setup new replication
        //           stream.
        let res = self.write_add_learner_entry(target, node).await;
        let log_id = match res {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(Err(e));
                return Ok(());
            }
        };

        // TODO(xp): nodes, i.e., replication streams, should also be a property of follower or candidate, for
        //           sending vote requests etc?
        let state = self.spawn_replication_stream(target).await;
        self.nodes.insert(target, state);

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
        for node_id in nodes {
            let repl_state = match self.nodes.get(node_id) {
                None => {
                    return Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound {
                        node_id: *node_id,
                    }));
                }
                Some(x) => x,
            };

            match expectation {
                None => {
                    // No expectation, whatever is OK.
                    continue;
                }
                Some(Expectation::AtLineRate) => {
                    // Expect to be at line rate but not.

                    let last_log_id = &self.core.engine.state.last_log_id();

                    if repl_state.is_line_rate(last_log_id, &self.core.config) {
                        continue;
                    }

                    let distance = last_log_id.next_index().saturating_sub(repl_state.matched.next_index());

                    let lagging = LearnerIsLagging {
                        node_id: *node_id,
                        matched: repl_state.matched,
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

    /// Handle the commitment of a uniform consensus cluster configuration.
    ///
    /// This is ony called by leader.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn handle_uniform_consensus_committed(&mut self, log_id: &LogId<C::NodeId>) {
        let index = log_id.index;

        // Step down if needed.
        if !self.core.engine.state.membership_state.effective.membership.is_voter(&self.core.id) {
            tracing::debug!("raft node is stepping down");

            // TODO(xp): transfer leadership
            self.core.set_target_state(ServerState::Learner);
            return;
        }

        let membership = &self.core.engine.state.membership_state.effective.membership;

        // remove nodes which not included in nodes and learners
        for (id, state) in self.nodes.iter_mut() {
            if membership.contains(id) {
                continue;
            }

            tracing::info!(
                "set remove_after_commit for {} = {}, membership: {:?}",
                id,
                index,
                self.core.engine.state.membership_state.effective
            );

            state.remove_since = Some(index)
        }

        let targets = self.nodes.keys().cloned().collect::<Vec<_>>();
        for target in targets {
            self.try_remove_replication(target).await;
        }

        self.core.engine.metrics_flags.set_replication_changed();
    }

    /// Remove a replication if the membership that does not include it has committed.
    ///
    /// Return true if removed.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn try_remove_replication(&mut self, target: C::NodeId) -> bool {
        tracing::debug!(target = display(target), "try_remove_replication");

        {
            let n = self.nodes.get(&target);

            if let Some(n) = n {
                if !self.need_to_remove_replication(n) {
                    return false;
                }
            } else {
                tracing::warn!("trying to remove absent replication to {}", target);
                return false;
            }
        }

        tracing::info!("removed replication to: {}", target);
        let repl_state = self.nodes.remove(&target);
        if let Some(s) = repl_state {
            let handle = s.repl_stream.handle;

            // Drop sender to notify the task to shutdown
            drop(s.repl_stream.repl_tx);

            tracing::debug!("joining removed replication: {}", target);
            let _x = handle.await;
            tracing::info!("Done joining removed replication : {}", target);
        }

        self.replication_metrics.update(RemoveTarget { target });
        // TODO(xp): set_replication_metrics_changed() can be removed.
        //           Use self.replication_metrics.version to detect changes.
        self.core.engine.metrics_flags.set_replication_changed();

        true
    }

    fn need_to_remove_replication(&self, node: &ReplicationState<C::NodeId>) -> bool {
        tracing::debug!(node=?node, "check if to remove a replication");

        let cfg = &self.core.config;
        let policy = &cfg.remove_replication;

        let st = &self.core.engine.state;
        let committed = st.committed;

        // `remove_since` is set only when the uniform membership log is committed.
        // Do not remove replication if it is not committed.
        let since = if let Some(since) = node.remove_since {
            since
        } else {
            return false;
        };

        if node.matched.index() >= Some(since) {
            tracing::debug!(
                node = debug(node),
                committed = debug(committed),
                "remove replication: uniform membership log committed and replicated to target"
            );
            return true;
        }

        match policy {
            RemoveReplicationPolicy::CommittedAdvance(n) => {
                // TODO(xp): test this. but not for now. It is meaningless without blank-log heartbeat.
                if committed.next_index() - since > *n {
                    tracing::debug!(
                        node = debug(node),
                        committed = debug(committed),
                        "remove replication: committed index is head of remove_since too much"
                    );
                    return true;
                }
            }
            RemoveReplicationPolicy::MaxNetworkFailures(n) => {
                if node.failures >= *n {
                    tracing::debug!(node = debug(node), "remove replication: too many replication failure");
                    return true;
                }
            }
        }

        false
    }
}
