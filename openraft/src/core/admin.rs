use std::collections::BTreeSet;
use std::mem::swap;
use std::option::Option::None;

use crate::config::RemoveReplicationPolicy;
use crate::core::replication_state::ReplicationState;
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
use crate::raft::ChangeMembers;
use crate::raft::ClientWriteResponse;
use crate::raft::RaftRespTx;
use crate::raft_types::LogIdOptionExt;
use crate::runtime::RaftRuntime;
use crate::versioned::Updatable;
use crate::EntryPayload;
use crate::LogId;
use crate::Node;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    // add node into learner,return true if the node is already a member or learner
    #[tracing::instrument(level = "debug", skip(self))]
    async fn write_add_learner_entry(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
    ) -> Result<(), AddLearnerError<C::NodeId>> {
        let curr = &self.core.engine.state.membership_state.effective.membership;
        let new_membership = curr.add_learner(target, node)?;

        tracing::debug!(?new_membership, "new_config");

        self.write_entry(EntryPayload::Membership(new_membership), None).await?;

        Ok(())
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
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn add_learner(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
        tx: RaftRespTx<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>>,
        blocking: bool,
    ) {
        tracing::debug!("add target node {} as learner {:?}", target, self.nodes.keys());

        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes already being synced, or in the nodes being removed.
        if target == self.core.id {
            tracing::debug!("target node is this node");
            let _ = tx.send(Ok(AddLearnerResponse {
                matched: self.core.engine.state.last_log_id,
            }));
            return;
        }

        let curr = &self.core.engine.state.membership_state.effective;
        let exists = curr.get_nodes().contains_key(&target);
        if exists {
            tracing::debug!("target {:?} already member or learner, can't add", target);

            if let Some(t) = self.nodes.get(&target) {
                tracing::debug!("target node is already a cluster member or is being synced");
                let _ = tx.send(Ok(AddLearnerResponse { matched: t.matched }));
                return;
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
        if let Err(e) = res {
            let _ = tx.send(Err(e));
            return;
        }

        if blocking {
            let state = self.spawn_replication_stream(target, Some(tx)).await;
            // TODO(xp): nodes, i.e., replication streams, should also be a property of follower or candidate, for
            //           sending vote requests etc?
            self.nodes.insert(target, state);
        } else {
            let state = self.spawn_replication_stream(target, None).await;
            self.nodes.insert(target, state);

            // non-blocking mode, do not know about the replication stat.
            let _ = tx.send(Ok(AddLearnerResponse { matched: None }));
        }

        tracing::debug!(
            "after add target node {} as learner {:?}",
            target,
            self.core.engine.state.last_log_id
        );
    }

    /// return true if there is pending uncommitted config change
    fn has_pending_config(&self) -> bool {
        // The last membership config is not committed yet.
        // Can not process the next one.
        self.core.engine.state.committed < self.core.engine.state.membership_state.effective.log_id
    }

    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        change_members: ChangeMembers<C::NodeId>,
        blocking: bool,
        turn_to_learner: bool,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>,
    ) -> Result<(), Fatal<C::NodeId>> {
        let members = change_members
            .apply_to(self.core.engine.state.membership_state.effective.membership.get_configs().last().unwrap());
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

        let curr = self.core.engine.state.membership_state.effective.membership.clone();
        let all_members = self.core.engine.state.membership_state.effective.all_members();
        let new_members = members.difference(all_members);

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

        if let Err(e) = self.are_nodes_at_line_rate(&new_members.cloned().collect::<BTreeSet<_>>(), blocking) {
            let _ = tx.send(Err(e));
            return Ok(());
        }

        self.write_entry(EntryPayload::Membership(new_config), Some(tx)).await?;
        Ok(())
    }

    /// return Ok if all the nodes is `is_line_rate`
    fn are_nodes_at_line_rate(
        &self,
        nodes: &BTreeSet<C::NodeId>,
        blocking: bool,
    ) -> Result<(), ClientWriteError<C::NodeId>> {
        // Check the proposed config for any new nodes. If ALL new nodes already have replication
        // streams AND are ready to join, then we can immediately proceed with entering joint
        // consensus. Else, new nodes need to first be brought up-to-speed.
        //
        // Here, all we do is check to see which nodes still need to be synced, which determines
        // if we can proceed.

        // TODO(xp): test change membership without adding as learner.

        // TODO(xp): 111 test adding a node that is not learner.
        // TODO(xp): 111 test adding a node that is lagging.
        for node_id in nodes.iter() {
            match self.nodes.get(node_id) {
                Some(node) => {
                    if node.is_line_rate(&self.core.engine.state.last_log_id, &self.core.config) {
                        // Node is ready to join.
                        continue;
                    }

                    if !blocking {
                        // Node has repl stream, but is not yet ready to join.
                        return Err(ClientWriteError::ChangeMembershipError(
                            ChangeMembershipError::LearnerIsLagging(LearnerIsLagging {
                                node_id: *node_id,
                                matched: node.matched,
                                distance: self
                                    .core
                                    .engine
                                    .state
                                    .last_log_id
                                    .next_index()
                                    .saturating_sub(node.matched.next_index()),
                            }),
                        ));
                    }
                }

                // Node does not yet have a repl stream, spawn one.
                None => {
                    return Err(ClientWriteError::ChangeMembershipError(
                        ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: *node_id }),
                    ));
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
    #[tracing::instrument(level = "debug", skip(self, payload, resp_tx), fields(id = display(self.core.id)))]
    pub async fn write_entry(
        &mut self,
        payload: EntryPayload<C>,
        resp_tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,
    ) -> Result<(), Fatal<C::NodeId>> {
        let mut entry_refs = [EntryRef::new(&payload)];
        // TODO: it should returns membership config error etc. currently this is done by the caller.
        self.core.engine.leader_append_entries(&mut entry_refs);

        // Install callback channels.
        if let Some(tx) = resp_tx {
            self.client_resp_channels.insert(entry_refs[0].log_id.index, tx);
        }

        self.run_engine_commands(&entry_refs).await?;

        Ok(())
    }

    async fn run_engine_commands<'p>(&mut self, input_entries: &[EntryRef<'p, C>]) -> Result<(), Fatal<C::NodeId>> {
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
        if !self.core.engine.state.membership_state.effective.membership.is_member(&self.core.id) {
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
