use std::collections::BTreeSet;
use std::sync::Arc;

use crate::core::client::ClientOrInternalResponseTx;
use crate::core::client::ClientRequestEntry;
use crate::core::ActiveMembership;
use crate::core::LeaderState;
use crate::core::NonVoterState;
use crate::core::State;
use crate::core::UpdateCurrentLeader;
use crate::error::AddNonVoterError;
use crate::error::ChangeConfigError;
use crate::error::InitializeError;
use crate::raft::ClientWriteRequest;
use crate::raft::MembershipConfig;
use crate::raft::RaftRespTx;
use crate::raft::RaftResponse;
use crate::replication::RaftEvent;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftError;
use crate::RaftNetwork;
use crate::RaftStorage;

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> NonVoterState<'a, D, R, N, S> {
    /// Handle the admin `init_with_config` command.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn handle_init_with_config(
        &mut self,
        mut members: BTreeSet<NodeId>,
    ) -> Result<(), InitializeError> {
        if self.core.last_log_id.index != 0 || self.core.current_term != 0 {
            tracing::error!({self.core.last_log_id.index, self.core.current_term}, "rejecting init_with_config request as last_log_index or current_term is 0");
            return Err(InitializeError::NotAllowed);
        }

        // Ensure given config contains this nodes ID as well.
        if !members.contains(&self.core.id) {
            members.insert(self.core.id);
        }

        // Build a new membership config from given init data & assign it as the new cluster
        // membership config in memory only.
        self.core.membership = ActiveMembership {
            log_id: LogId { term: 1, index: 1 },
            membership: MembershipConfig {
                members,
                members_after_consensus: None,
            },
        };

        // Become a candidate and start campaigning for leadership. If this node is the only node
        // in the cluster, then become leader without holding an election. If members len == 1, we
        // know it is our ID due to the above code where we ensure our own ID is present.
        if self.core.membership.membership.members.len() == 1 {
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
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) fn add_member(
        &mut self,
        target: NodeId,
        tx: RaftRespTx<RaftResponse, AddNonVoterError>,
        blocking: bool,
    ) {
        // TODO(xp): 111 a blocking change_membership can be done in Raft::change_membership: it add the replication
        //           stream and wait for it to become line-rate.

        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes alreading being synced, or in the nodes being removed.
        if target == self.core.id || self.nodes.contains_key(&target) {
            tracing::debug!("target node is already a cluster member or is being synced");
            let _ = tx.send(Ok(RaftResponse::NoChange));
            return;
        }

        if blocking {
            let state = self.spawn_replication_stream(target, Some(tx));
            self.nodes.insert(target, state);
        } else {
            let state = self.spawn_replication_stream(target, None);
            self.nodes.insert(target, state);

            // non-blocking mode, do not know about the replication stat.
            let _ = tx.send(Ok(RaftResponse::LogId {
                log_id: LogId { term: 0, index: 0 },
            }));
        }
    }

    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        members: BTreeSet<NodeId>,
        blocking: bool,
        tx: RaftRespTx<RaftResponse, ChangeConfigError>,
    ) {
        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ChangeConfigError::InoperableConfig));
            return;
        }

        // The last membership config is not committed yet.
        // Can not process the next one.
        if self.core.commit_index < self.core.membership.log_id.index {
            let _ = tx.send(Err(ChangeConfigError::ConfigChangeInProgress {
                membership_log_id: self.core.membership.log_id,
            }));
            return;
        }

        let new_config;

        let curr = &self.core.membership.membership;

        if let Some(ref next_membership) = curr.members_after_consensus {
            // When it is in joint state, it is only allowed to change to the `members_after_consensus`
            if &members != next_membership {
                let _ = tx.send(Err(ChangeConfigError::Incompatible {
                    curr: curr.clone(),
                    to: members,
                }));
                return;
            } else {
                new_config = MembershipConfig {
                    members: next_membership.clone(),
                    members_after_consensus: None,
                };
            }
        } else {
            // currently it is uniform config, enter joint state
            new_config = MembershipConfig {
                members: curr.members.clone(),
                members_after_consensus: Some(members.clone()),
            };
        }

        // Check the proposed config for any new nodes. If ALL new nodes already have replication
        // streams AND are ready to join, then we can immediately proceed with entering joint
        // consensus. Else, new nodes need to first be brought up-to-speed.
        //
        // Here, all we do is check to see which nodes still need to be synced, which determines
        // if we can proceed.

        // TODO(xp): test change membership without adding as non-voter.

        // TODO(xp): 111 test adding a node that is not non-voter.
        // TODO(xp): 111 test adding a node that is lagging.
        for new_node in members.difference(&self.core.membership.membership.members) {
            match self.nodes.get(&new_node) {
                // Node is ready to join.
                Some(node) => {
                    if node.is_line_rate(&self.core.last_log_id, &self.core.config) {
                        continue;
                    }

                    if !blocking {
                        // Node has repl stream, but is not yet ready to join.
                        let _ = tx.send(Err(ChangeConfigError::NonVoterIsLagging {
                            node_id: *new_node,
                            distance: self.core.last_log_id.index.saturating_sub(node.matched.index),
                        }));
                        return;
                    }
                }

                // Node does not yet have a repl stream, spawn one.
                None => {
                    // TODO(xp): 111 distance
                    let _ = tx.send(Err(ChangeConfigError::NonVoterNotFound { node_id: *new_node }));
                    return;
                }
            }
        }

        // TODO(xp): 111 report metrics?
        let res = self.append_membership_log(new_config, Some(tx)).await;

        if let Err(e) = res {
            tracing::error!("append joint log error: {:?}", e);
        }
    }

    #[tracing::instrument(level = "debug", skip(self, resp_tx), fields(id=self.core.id))]
    pub async fn append_membership_log(
        &mut self,
        mem: MembershipConfig,
        resp_tx: Option<RaftRespTx<RaftResponse, ChangeConfigError>>,
    ) -> Result<(), RaftError> {
        let payload = ClientWriteRequest::<D>::new_config(mem.clone());
        let res = self.append_payload_to_log(payload.entry).await;

        // Caveat: membership must be updated before commit check is done with the new config.
        self.core.membership = ActiveMembership {
            log_id: self.core.last_log_id,
            membership: mem,
        };

        self.leader_report_metrics();

        let entry = match res {
            Ok(entry) => entry,
            Err(err) => {
                let err_str = err.to_string();
                if let Some(tx) = resp_tx {
                    let send_res = tx.send(Err(err.into()));
                    if let Err(e) = send_res {
                        tracing::error!("send response res error: {:?}", e);
                    }
                }
                return Err(RaftError::RaftStorage(anyhow::anyhow!(err_str)));
            }
        };

        let cr_entry = ClientRequestEntry {
            entry: Arc::new(entry),
            tx: ClientOrInternalResponseTx::Internal(resp_tx),
        };

        self.replicate_client_request(cr_entry).await;

        Ok(())
    }

    /// Handle the commitment of a uniform consensus cluster configuration.
    ///
    /// This is ony called by leader.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) fn handle_uniform_consensus_committed(&mut self, log_id: &LogId) {
        let index = log_id.index;
        // TODO(xp): 111 when membership config log is committed, there is nothing has to do.
        // TODO(xp): removed follower should be able to receive the message that commits a joint log.

        // Step down if needed.
        if !self.core.membership.membership.contains(&self.core.id) {
            // }
            // if self.is_stepping_down {
            tracing::debug!("raft node is stepping down");
            self.core.set_target_state(State::NonVoter);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            return;
        }

        // Remove any replication streams which have replicated this config & which are no longer
        // cluster members. All other replication streams which are no longer cluster members, but
        // which have not yet replicated this config will be marked for removal.
        let membership = &self.core.membership.membership;
        let nodes_to_remove: Vec<_> = self
            .nodes
            .iter_mut()
            .filter(|(id, _)| !membership.contains(id))
            .filter_map(|(idx, repl_state)| {
                if repl_state.matched.index >= index {
                    Some(*idx)
                } else {
                    repl_state.remove_after_commit = Some(index);
                    None
                }
            })
            .collect();

        let follower_ids: Vec<u64> = self.nodes.keys().cloned().collect();
        tracing::debug!("nodes: {:?}", follower_ids);
        tracing::debug!("membership: {:?}", self.core.membership);
        tracing::debug!("nodes_to_remove: {:?}", nodes_to_remove);

        for target in nodes_to_remove {
            tracing::debug!(target, "removing target node from replication pool");
            // TODO(xp): just drop the replication then the task will be terminated.
            let removed = self.nodes.remove(&target);
            assert!(removed.is_some());

            tracing::info!(
                "handle_uniform_consensus_committed: removed replication node: {} {:?}",
                target,
                removed.as_ref().map(|x| (*x).summary())
            );

            if let Some(node) = removed {
                let _ = node.replstream.repl_tx.send((RaftEvent::Terminate, tracing::debug_span!("CH")));
                self.leader_metrics.replication.remove(&target);
            }
        }
        self.leader_report_metrics();
    }
}
