use std::collections::BTreeSet;
use std::sync::Arc;

use crate::core::client::ClientRequestEntry;
use crate::core::EffectiveMembership;
use crate::core::LeaderState;
use crate::core::LearnerState;
use crate::core::State;
use crate::core::UpdateCurrentLeader;
use crate::error::AddLearnerError;
use crate::error::ChangeMembershipError;
use crate::error::ClientWriteError;
use crate::error::InitializeError;
use crate::raft::AddLearnerResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::EntryPayload;
use crate::raft::RaftRespTx;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::Membership;
use crate::NodeId;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::StorageError;

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LearnerState<'a, D, R, N, S> {
    /// Handle the admin `init_with_config` command.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn handle_init_with_config(
        &mut self,
        mut members: BTreeSet<NodeId>,
    ) -> Result<(), InitializeError> {
        if self.core.last_log_id.is_none() || self.core.current_term != 0 {
            tracing::error!(
                { last_log_id=?self.core.last_log_id, self.core.current_term },
                "rejecting init_with_config request as last_log_index is not None or current_term is 0");
            return Err(InitializeError::NotAllowed);
        }

        // Ensure given config contains this nodes ID as well.
        if !members.contains(&self.core.id) {
            members.insert(self.core.id);
        }

        // Build a new membership config from given init data & assign it as the new cluster
        // membership config in memory only.
        self.core.effective_membership = EffectiveMembership {
            log_id: LogId { term: 1, index: 1 },
            membership: Membership::new_single(members),
        };

        // Become a candidate and start campaigning for leadership. If this node is the only node
        // in the cluster, then become leader without holding an election. If members len == 1, we
        // know it is our ID due to the above code where we ensure our own ID is present.
        if self.core.effective_membership.membership.all_nodes().len() == 1 {
            self.core.current_term += 1;
            self.core.voted_for = Some(self.core.id);

            // TODO(xp): it should always commit a initial log entry
            self.core.set_target_state(State::Leader);
            self.core.save_hard_state().await?;
        } else {
            self.core.set_target_state(State::Candidate);
        }

        Ok(())
    }
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    #[tracing::instrument(level = "debug", skip(self))]
    async fn add_learner_into_membership(&mut self, target: &NodeId) -> bool {
        tracing::debug!(
            "before add_learner_into_membership target node {} into learner {:?}",
            target,
            self.nodes.keys()
        );

        let curr = &mut self.core.effective_membership.membership;
        if curr.contain_learner(&target) {
            tracing::debug!("target node {} is already a learner", target);

            return false;
        }
        curr.add_learner(target);

        // append_membership_log after learner has added into nodes
        let new_config = self.core.effective_membership.membership.clone();

        tracing::debug!(?new_config, "new_config");

        let _ = self.append_membership_log(new_config, None).await;

        return true;
    }

    /// Add a new node to the cluster as a learner, bringing it up-to-speed, and then responding
    /// on the given channel.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn add_learner(
        &mut self,
        target: NodeId,
        tx: RaftRespTx<AddLearnerResponse, AddLearnerError>,
        blocking: bool,
    ) {
        tracing::debug!("add target node {} as learner {:?}", target, self.nodes.keys());

        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes already being synced, or in the nodes being removed.
        if target == self.core.id {
            tracing::debug!("target node is this node");
            let _ = tx.send(Ok(AddLearnerResponse {
                matched: self.core.last_log_id.expect("raft core is uninitialized."),
            }));
            return;
        }

        if let Some(t) = self.nodes.get(&target) {
            tracing::debug!("target node is already a cluster member or is being synced");
            let _ = tx.send(Ok(AddLearnerResponse { matched: t.matched }));
            return;
        }

        let _ = self.add_learner_into_membership(&target).await;

        if blocking {
            let state = self.spawn_replication_stream(target, Some(tx));
            self.nodes.insert(target, state);
        } else {
            let state = self.spawn_replication_stream(target, None);
            self.nodes.insert(target, state);

            // non-blocking mode, do not know about the replication stat.
            let _ = tx.send(Ok(AddLearnerResponse {
                matched: LogId::new(0, 0),
            }));
        }

        tracing::debug!(
            "after add target node {} as learner {:?}",
            target,
            self.core.last_log_id
        );
    }

    pub fn has_pending_membership_change(&self, members: &BTreeSet<NodeId>) -> bool {
        if self.core.committed >= self.core.effective_membership.log_id {
            return false;
        }

        let curr = &self.core.effective_membership.membership;
        let all_learners = curr.all_learners();
        for new_node in members.difference(curr.all_nodes()) {
            match all_learners.get(new_node) {
                Some(_node) => {
                    continue;
                }
                None => {
                    // if new member not include in the learners, there is pending membership change
                    return true;
                }
            }
        }
        return false;
    }

    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        members: BTreeSet<NodeId>,
        blocking: bool,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    ) -> Result<(), StorageError> {
        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::EmptyMembership,
            )));
            return Ok(());
        }

        // The last membership config is not committed yet.
        // Can not process the next one.
        if self.has_pending_membership_change(&members) {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::InProgress {
                    membership_log_id: self.core.effective_membership.log_id,
                },
            )));
            return Ok(());
        }

        let curr = &self.core.effective_membership.membership;

        let mut new_config = curr.next_safe(members.clone());

        tracing::debug!(?new_config, "new_config");

        // Check the proposed config for any new nodes. If ALL new nodes already have replication
        // streams AND are ready to join, then we can immediately proceed with entering joint
        // consensus. Else, new nodes need to first be brought up-to-speed.
        //
        // Here, all we do is check to see which nodes still need to be synced, which determines
        // if we can proceed.

        // TODO(xp): test change membership without adding as learner.

        // TODO(xp): 111 test adding a node that is not learner.
        // TODO(xp): 111 test adding a node that is lagging.
        for new_node in members.difference(curr.all_nodes()) {
            match self.nodes.get(new_node) {
                Some(node) => {
                    if node.is_line_rate(&self.core.last_log_id.unwrap_or_default(), &self.core.config) {
                        // Node is ready to join, remove from learners
                        new_config.remove_learner(new_node);
                        continue;
                    }

                    if !blocking {
                        // Node has repl stream, but is not yet ready to join.
                        let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                            ChangeMembershipError::LearnerIsLagging {
                                node_id: *new_node,
                                matched: node.matched,
                                distance: self.core.last_log_id.unwrap().index.saturating_sub(node.matched.index),
                            },
                        )));
                        return Ok(());
                    }
                }

                // Node does not yet have a repl stream, spawn one.
                None => {
                    let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                        ChangeMembershipError::LearnerNotFound { node_id: *new_node },
                    )));
                    return Ok(());
                }
            }
        }

        self.append_membership_log(new_config, Some(tx)).await?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, resp_tx), fields(id=self.core.id))]
    pub async fn append_membership_log(
        &mut self,
        mem: Membership,
        resp_tx: Option<RaftRespTx<ClientWriteResponse<R>, ClientWriteError>>,
    ) -> Result<(), StorageError> {
        let payload = EntryPayload::Membership(mem.clone());
        let entry = self.append_payload_to_log(payload).await?;

        // Caveat: membership must be updated before commit check is done with the new config.
        self.core.effective_membership = EffectiveMembership {
            log_id: entry.log_id,
            membership: mem,
        };

        self.leader_report_metrics();

        let cr_entry = ClientRequestEntry {
            entry: Arc::new(entry),
            tx: resp_tx,
        };

        self.replicate_client_request(cr_entry).await?;

        Ok(())
    }

    /// Handle the commitment of a uniform consensus cluster configuration.
    ///
    /// This is ony called by leader.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) fn handle_uniform_consensus_committed(&mut self, log_id: &LogId) {
        let index = log_id.index;

        // Step down if needed.
        if !self.core.effective_membership.membership.contains(&self.core.id) {
            tracing::debug!("raft node is stepping down");

            // TODO(xp): transfer leadership
            self.core.set_target_state(State::Learner);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            return;
        }

        let membership = &self.core.effective_membership.membership;

        // remove nodes which not included in nodes, nor learners
        let all_nodes = membership.all_nodes();
        let all_learners = membership.all_learners();
        for (id, state) in self.nodes.iter_mut() {
            if all_nodes.contains(id) || all_learners.contains(id) {
                continue;
            }

            tracing::info!(
                "set remove_after_commit for {} = {}, membership: {:?}",
                id,
                index,
                self.core.effective_membership
            );

            state.remove_since = Some(index)
        }

        let targets = self.nodes.keys().cloned().collect::<Vec<_>>();
        for target in targets {
            self.try_remove_replication(target);
        }

        self.leader_report_metrics();
    }

    /// Remove a replication if the membership that does not include it has committed.
    ///
    /// Return true if removed.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn try_remove_replication(&mut self, target: u64) -> bool {
        tracing::debug!(target, "try_remove_replication");

        {
            let n = self.nodes.get(&target);

            if let Some(n) = n {
                if let Some(since) = n.remove_since {
                    if n.matched.index < since {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                tracing::warn!("trying to remove absent replication to {}", target);
                return false;
            }
        }

        tracing::info!("removed replication to: {}", target);
        self.nodes.remove(&target);
        self.leader_metrics.replication.remove(&target);
        true
    }
}
