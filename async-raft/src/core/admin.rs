use std::collections::HashSet;

use futures::future::{FutureExt, TryFutureExt};
use tokio::sync::oneshot;

use crate::core::client::ClientRequestEntry;
use crate::core::{ConsensusState, LeaderState, NonVoterReplicationState, NonVoterState, State, UpdateCurrentLeader};
use crate::error::{ChangeConfigError, InitializeError, RaftError};
use crate::raft::{ChangeMembershipTx, ClientWriteRequest, MembershipConfig};
use crate::replication::RaftEvent;
use crate::{AppData, AppDataResponse, NodeId, RaftNetwork, RaftStorage};

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> NonVoterState<'a, D, R, N, S> {
    /// Handle the admin `init_with_config` command.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) async fn handle_init_with_config(&mut self, mut members: HashSet<NodeId>) -> Result<(), InitializeError> {
        if self.core.last_log_index != 0 || self.core.current_term != 0 {
            tracing::error!({self.core.last_log_index, self.core.current_term}, "rejecting init_with_config request as last_log_index or current_term is 0");
            return Err(InitializeError::NotAllowed);
        }

        // Ensure given config contains this nodes ID as well.
        if !members.contains(&self.core.id) {
            members.insert(self.core.id);
        }

        // Build a new membership config from given init data & assign it as the new cluster
        // membership config in memory only.
        self.core.membership = MembershipConfig {
            members,
            members_after_consensus: None,
        };

        // Become a candidate and start campaigning for leadership. If this node is the only node
        // in the cluster, then become leader without holding an election. If members len == 1, we
        // know it is our ID due to the above code where we ensure our own ID is present.
        if self.core.membership.members.len() == 1 {
            self.core.current_term += 1;
            self.core.voted_for = Some(self.core.id);
            self.core.set_target_state(State::Leader);
            self.core.save_hard_state().await?;
        } else {
            self.core.set_target_state(State::Candidate);
        }

        Ok(())
    }
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    /// Add a new node to the cluster as a non-voter, bringing it up-to-speed, and then responding
    /// on the given channel.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(super) fn add_member(&mut self, target: NodeId, tx: oneshot::Sender<Result<(), ChangeConfigError>>) {
        // Ensure the node doesn't already exist in the current config, in the set of new nodes
        // alreading being synced, or in the nodes being removed.
        if self.core.membership.members.contains(&target)
            || self
                .core
                .membership
                .members_after_consensus
                .as_ref()
                .map(|new| new.contains(&target))
                .unwrap_or(false)
            || self.non_voters.contains_key(&target)
        {
            tracing::debug!("target node is already a cluster member or is being synced");
            let _ = tx.send(Err(ChangeConfigError::Noop));
            return;
        }

        // Spawn a replication stream for the new member. Track state as a non-voter so that it
        // can be updated to be added to the cluster config once it has been brought up-to-date.
        let state = self.spawn_replication_stream(target);
        self.non_voters.insert(
            target,
            NonVoterReplicationState {
                state,
                is_ready_to_join: false,
                tx: Some(tx),
            },
        );
    }

    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(super) async fn change_membership(&mut self, members: HashSet<NodeId>, tx: ChangeMembershipTx) {
        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ChangeConfigError::InoperableConfig));
            return;
        }

        // Only allow config updates when currently in a uniform consensus state.
        match &self.consensus_state {
            ConsensusState::Uniform => (),
            ConsensusState::NonVoterSync { .. } | ConsensusState::Joint { .. } => {
                let _ = tx.send(Err(ChangeConfigError::ConfigChangeInProgress));
                return;
            }
        }

        // Check the proposed config for any new nodes. If ALL new nodes already have replication
        // streams AND are ready to join, then we can immediately proceed with entering joint
        // consensus. Else, new nodes need to first be brought up-to-speed.
        //
        // Here, all we do is check to see which nodes still need to be synced, which determines
        // if we can proceed.
        let mut awaiting = HashSet::new();
        for new_node in members.difference(&self.core.membership.members) {
            match self.non_voters.get(&new_node) {
                // Node is ready to join.
                Some(node) if node.is_ready_to_join => continue,
                // Node has repl stream, but is not yet ready to join.
                Some(_) => (),
                // Node does not yet have a repl stream, spawn one.
                None => {
                    // Spawn a replication stream for the new member. Track state as a non-voter so that it
                    // can be updated to be added to the cluster config once it has been brought up-to-date.
                    let state = self.spawn_replication_stream(*new_node);
                    self.non_voters.insert(
                        *new_node,
                        NonVoterReplicationState {
                            state,
                            is_ready_to_join: false,
                            tx: None,
                        },
                    );
                }
            }
            awaiting.insert(*new_node);
        }
        // If there are new nodes which need to sync, then we need to wait until they are synced.
        // Once they've finished, this routine will be called again to progress further.
        if !awaiting.is_empty() {
            self.consensus_state = ConsensusState::NonVoterSync { awaiting, members, tx };
            return;
        }

        // Enter into joint consensus if we are not awaiting any new nodes.
        if !members.contains(&self.core.id) {
            self.is_stepping_down = true;
        }
        self.consensus_state = ConsensusState::Joint { is_committed: false };
        self.core.membership.members_after_consensus = Some(members);

        // Propagate the command as any other client request.
        let payload = ClientWriteRequest::<D>::new_config(self.core.membership.clone());
        let (tx_joint, rx_join) = oneshot::channel();
        let entry = match self.append_payload_to_log(payload.entry).await {
            Ok(entry) => entry,
            Err(err) => {
                let _ = tx.send(Err(err.into()));
                return;
            }
        };
        let cr_entry = ClientRequestEntry::from_entry(entry, tx_joint);
        self.replicate_client_request(cr_entry).await;
        self.core.report_metrics();

        // Setup channels for eventual response to the 2-phase config change.
        let (tx_cfg_change, rx_cfg_change) = oneshot::channel();
        self.propose_config_change_cb = Some(tx_cfg_change); // Once the entire process is done, this is our response channel.
        self.joint_consensus_cb.push(rx_join); // Receiver for when the joint consensus is committed.
        tokio::spawn(async move {
            let res = rx_cfg_change
                .map_err(|_| RaftError::ShuttingDown)
                .into_future()
                .then(|res| {
                    futures::future::ready(match res {
                        Ok(Ok(_)) => Ok(()),
                        Ok(Err(err)) => Err(ChangeConfigError::from(err)),
                        Err(err) => Err(ChangeConfigError::from(err)),
                    })
                })
                .await;
            let _ = tx.send(res);
        });
    }

    /// Handle the commitment of a joint consensus cluster configuration.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) async fn handle_joint_consensus_committed(&mut self) -> Result<(), RaftError> {
        if let ConsensusState::Joint { is_committed, .. } = &mut self.consensus_state {
            *is_committed = true; // Mark as comitted.
        }
        // Only proceed to finalize this joint consensus if there are no remaining nodes being synced.
        if self.consensus_state.is_joint_consensus_safe_to_finalize() {
            self.finalize_joint_consensus().await?;
        }
        Ok(())
    }

    /// Finalize the comitted joint consensus.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) async fn finalize_joint_consensus(&mut self) -> Result<(), RaftError> {
        // Only proceed if it is safe to do so.
        if !self.consensus_state.is_joint_consensus_safe_to_finalize() {
            tracing::error!("attempted to finalize joint consensus when it was not safe to do so");
            return Ok(());
        }

        // Cut the cluster config over to the new membership config.
        if let Some(new_members) = self.core.membership.members_after_consensus.take() {
            self.core.membership.members = new_members;
        }
        self.consensus_state = ConsensusState::Uniform;

        // NOTE WELL: this implementation uses replication streams (src/replication/**) to replicate
        // entries. Nodes which do not exist in the new config will still have an active replication
        // stream until the current leader determines that they have replicated the config entry which
        // removes them from the cluster. At that point in time, the node will revert to non-voter state.
        //
        // HOWEVER, if an election takes place, the new leader will not have the old nodes in its config
        // and the old nodes may not revert to non-voter state using the above mechanism. That is fine.
        // The Raft spec accounts for this using the 3rd safety measure of cluster configuration changes
        // described at the very end of §6. This measure is already implemented and in place.

        // Propagate the next command as any other client request.
        let payload = ClientWriteRequest::<D>::new_config(self.core.membership.clone());
        let (tx_uniform, rx_uniform) = oneshot::channel();
        let entry = self.append_payload_to_log(payload.entry).await?;
        let cr_entry = ClientRequestEntry::from_entry(entry, tx_uniform);
        self.replicate_client_request(cr_entry).await;
        self.core.report_metrics();

        // Setup channel for eventual commitment of the uniform consensus config.
        self.uniform_consensus_cb.push(rx_uniform); // Receiver for when the uniform consensus is committed.
        Ok(())
    }

    /// Handle the commitment of a uniform consensus cluster configuration.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) async fn handle_uniform_consensus_committed(&mut self, index: u64) -> Result<(), RaftError> {
        // Step down if needed.
        if self.is_stepping_down {
            tracing::debug!("raft node is stepping down");
            self.core.set_target_state(State::NonVoter);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            return Ok(());
        }

        // Remove any replication streams which have replicated this config & which are no longer
        // cluster members. All other replication streams which are no longer cluster members, but
        // which have not yet replicated this config will be marked for removal.
        let membership = &self.core.membership;
        let nodes_to_remove: Vec<_> = self
            .nodes
            .iter_mut()
            .filter(|(id, _)| !membership.contains(id))
            .filter_map(|(idx, replstate)| {
                if replstate.match_index >= index {
                    Some(*idx)
                } else {
                    replstate.remove_after_commit = Some(index);
                    None
                }
            })
            .collect();
        for node in nodes_to_remove {
            tracing::debug!({ target = node }, "removing target node from replication pool");
            if let Some(node) = self.nodes.remove(&node) {
                let _ = node.replstream.repltx.send(RaftEvent::Terminate);
            }
        }
        self.core.report_metrics();
        Ok(())
    }
}
