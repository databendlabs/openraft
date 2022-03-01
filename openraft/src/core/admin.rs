use std::collections::BTreeSet;
use std::option::Option::None;
use std::sync::Arc;

use tracing::warn;

use crate::core::client::ClientRequestEntry;
use crate::core::LeaderState;
use crate::core::LearnerState;
use crate::core::State;
use crate::error::AddLearnerError;
use crate::error::ChangeMembershipError;
use crate::error::ClientWriteError;
use crate::error::EmptyMembership;
use crate::error::InProgress;
use crate::error::InitializeError;
use crate::error::LearnerIsLagging;
use crate::error::LearnerNotFound;
use crate::error::NodeIdNotInNodes;
use crate::membership::EitherNodesOrIds;
use crate::raft::AddLearnerResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::EntryPayload;
use crate::raft::RaftRespTx;
use crate::raft_types::LogIdOptionExt;
use crate::LogId;
use crate::Membership;
use crate::Node;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LearnerState<'a, C, N, S> {
    /// Handle the admin `init_with_config` command.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn handle_init_with_config(
        &mut self,
        members: EitherNodesOrIds<C>,
    ) -> Result<(), InitializeError<C>> {
        // TODO(xp): simplify this condition

        if self.core.last_log_id.is_some() || self.core.vote.term != 0 {
            tracing::error!(
                last_log_id=?self.core.last_log_id, ?self.core.vote,
                "rejecting init_with_config request as last_log_index is not None or current_term is not 0");
            return Err(InitializeError::NotAllowed);
        }

        let node_ids = members.node_ids();

        if !node_ids.contains(&self.core.id) {
            let e = NodeIdNotInNodes {
                node_id: self.core.id,
                node_ids,
            };
            return Err(InitializeError::NodeNotInCluster(e));
        }

        let membership = Membership::new(vec![node_ids], None).set_nodes(members.nodes())?;

        let payload = EntryPayload::Membership(membership.clone());
        let _ent = self.core.append_payload_to_log(payload).await?;

        self.core.set_target_state(State::Candidate);

        Ok(())
    }
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    // add node into learner,return true if the node is already a member or learner
    #[tracing::instrument(level = "debug", skip(self))]
    async fn add_learner_into_membership(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
    ) -> Result<bool, NodeIdNotInNodes<C>> {
        tracing::debug!(
            "add_learner_into_membership target node {:?} into learner {:?}",
            target,
            self.nodes.keys()
        );

        let curr = &self.core.effective_membership.membership;

        let new_membership = curr.add_learner(target, node)?;

        if new_membership.all_learners() == curr.all_learners() {
            tracing::debug!("target {:?} already member or learner, can't add", target);
            return Ok(true);
        }

        tracing::debug!(?new_membership, "new_config");

        let _ = self.append_membership_log(new_membership, None).await;

        Ok(false)
    }

    /// Add a new node to the cluster as a learner, bringing it up-to-speed, and then responding
    /// on the given channel.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn add_learner(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
        tx: RaftRespTx<AddLearnerResponse<C>, AddLearnerError<C>>,
        blocking: bool,
    ) {
        tracing::debug!("add target node {} as learner {:?}", target, self.nodes.keys());

        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes already being synced, or in the nodes being removed.
        if target == self.core.id {
            tracing::debug!("target node is this node");
            let _ = tx.send(Ok(AddLearnerResponse::<C> {
                matched: self.core.last_log_id,
            }));
            return;
        }

        if let Some(t) = self.nodes.get(&target) {
            tracing::debug!("target node is already a cluster member or is being synced");
            let _ = tx.send(Ok(AddLearnerResponse { matched: t.matched }));
            return;
        }

        let exist = self.add_learner_into_membership(target, node).await;
        let exist = match exist {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(Err(AddLearnerError::<C>::from(e)));
                return;
            }
        };

        if exist {
            return;
        }

        if blocking {
            let state = self.spawn_replication_stream(target, Some(tx)).await;
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
            self.core.last_log_id
        );
    }

    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        members: BTreeSet<C::NodeId>,
        blocking: bool,
        turn_to_learner: bool,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C>>,
    ) -> Result<(), StorageError<C>> {
        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::EmptyMembership(EmptyMembership {}),
            )));
            return Ok(());
        }

        // The last membership config is not committed yet.
        // Can not process the next one.
        if self.core.committed < Some(self.core.effective_membership.log_id) {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::InProgress(InProgress {
                    membership_log_id: self.core.effective_membership.log_id,
                }),
            )));
            return Ok(());
        }

        let curr = self.core.effective_membership.membership.clone();
        let all_members = self.core.effective_membership.all_members();
        let new_members = members.difference(all_members);

        let new_config = {
            let res = curr.next_safe(members.clone(), turn_to_learner);
            match res {
                Ok(x) => x,
                Err(e) => {
                    let change_err = ChangeMembershipError::NodeNotInCluster(e);
                    let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(change_err)));
                    return Ok(());
                }
            }
        };

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
        for new_node in new_members {
            match self.nodes.get(new_node) {
                Some(node) => {
                    if node.is_line_rate(&self.core.last_log_id, &self.core.config) {
                        // Node is ready to join.
                        continue;
                    }

                    if !blocking {
                        // Node has repl stream, but is not yet ready to join.
                        let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                            ChangeMembershipError::LearnerIsLagging(LearnerIsLagging {
                                node_id: *new_node,
                                matched: node.matched,
                                distance: self.core.last_log_id.next_index().saturating_sub(node.matched.next_index()),
                            }),
                        )));
                        return Ok(());
                    }
                }

                // Node does not yet have a repl stream, spawn one.
                None => {
                    let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                        ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: *new_node }),
                    )));
                    return Ok(());
                }
            }
        }

        self.append_membership_log(new_config, Some(tx)).await?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, resp_tx), fields(id=display(self.core.id)))]
    pub async fn append_membership_log(
        &mut self,
        mem: Membership<C>,
        resp_tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C>>>,
    ) -> Result<(), StorageError<C>> {
        let payload = EntryPayload::Membership(mem.clone());
        let entry = self.core.append_payload_to_log(payload).await?;

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
    pub(super) fn handle_uniform_consensus_committed(&mut self, log_id: &LogId<C>) {
        let index = log_id.index;

        // Step down if needed.
        if !self.core.effective_membership.membership.is_member(&self.core.id) {
            tracing::debug!("raft node is stepping down");

            // TODO(xp): transfer leadership
            self.core.set_target_state(State::Learner);
            return;
        }

        let membership = &self.core.effective_membership.membership;

        // remove nodes which not included in nodes and learners
        for (id, state) in self.nodes.iter_mut() {
            if membership.contains(id) {
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
    pub fn try_remove_replication(&mut self, target: C::NodeId) -> bool {
        tracing::debug!(target = display(target), "try_remove_replication");

        {
            let n = self.nodes.get(&target);

            if let Some(n) = n {
                if let Some(since) = n.remove_since {
                    if n.matched.index() < Some(since) {
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
        let mut metrics_clone = self.leader_metrics.as_ref().clone();
        metrics_clone.replication.remove(&target);
        self.leader_metrics = Arc::new(metrics_clone);
        true
    }
}
