use std::collections::BTreeSet;
use std::sync::Arc;

use crate::core::client::ClientRequestEntry;
use crate::core::EffectiveMembership;
use crate::core::LeaderState;
use crate::core::NonVoterState;
use crate::core::State;
use crate::core::UpdateCurrentLeader;
use crate::error::AddNonVoterError;
use crate::error::ChangeMembershipError;
use crate::error::ClientWriteError;
use crate::error::InitializeError;
use crate::raft::AddNonVoterResponse;
use crate::raft::ClientWriteRequest;
use crate::raft::ClientWriteResponse;
use crate::raft::MembershipConfig;
use crate::raft::RaftRespTx;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
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
        self.core.membership = EffectiveMembership {
            log_id: LogId { term: 1, index: 1 },
            membership: MembershipConfig::new_single(members),
        };

        // Become a candidate and start campaigning for leadership. If this node is the only node
        // in the cluster, then become leader without holding an election. If members len == 1, we
        // know it is our ID due to the above code where we ensure our own ID is present.
        if self.core.membership.membership.all_nodes().len() == 1 {
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
    pub(super) fn add_non_voter(
        &mut self,
        target: NodeId,
        tx: RaftRespTx<AddNonVoterResponse, AddNonVoterError>,
        blocking: bool,
    ) {
        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes already being synced, or in the nodes being removed.
        if target == self.core.id {
            tracing::debug!("target node is this node");
            let _ = tx.send(Ok(AddNonVoterResponse {
                matched: self.core.last_log_id,
            }));
            return;
        }

        if let Some(t) = self.nodes.get(&target) {
            tracing::debug!("target node is already a cluster member or is being synced");
            let _ = tx.send(Ok(AddNonVoterResponse { matched: t.matched }));
            return;
        }

        if blocking {
            let state = self.spawn_replication_stream(target, Some(tx));
            self.nodes.insert(target, state);
        } else {
            let state = self.spawn_replication_stream(target, None);
            self.nodes.insert(target, state);

            // non-blocking mode, do not know about the replication stat.
            let _ = tx.send(Ok(AddNonVoterResponse {
                matched: LogId::new(0, 0),
            }));
        }
    }

    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        members: BTreeSet<NodeId>,
        blocking: bool,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    ) {
        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::EmptyMembership,
            )));
            return;
        }

        // The last membership config is not committed yet.
        // Can not process the next one.
        if self.core.commit_index < self.core.membership.log_id.index {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::InProgress {
                    membership_log_id: self.core.membership.log_id,
                },
            )));
            return;
        }

        let new_config;

        let curr = &self.core.membership.membership;

        if let Some(next_membership) = curr.get_ith_config(1) {
            // When it is in joint state, it is only allowed to change to the `members_after_consensus`
            if &members != next_membership {
                let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                    ChangeMembershipError::Incompatible {
                        curr: curr.clone(),
                        to: members,
                    },
                )));
                return;
            } else {
                new_config = MembershipConfig::new_single(next_membership.clone());
            }
        } else {
            // currently it is uniform config, enter joint state
            new_config = MembershipConfig::new_multi(vec![curr.get_ith_config(0).unwrap().clone(), members.clone()]);
        }

        tracing::debug!(?new_config, "new_config");

        // Check the proposed config for any new nodes. If ALL new nodes already have replication
        // streams AND are ready to join, then we can immediately proceed with entering joint
        // consensus. Else, new nodes need to first be brought up-to-speed.
        //
        // Here, all we do is check to see which nodes still need to be synced, which determines
        // if we can proceed.

        // TODO(xp): test change membership without adding as non-voter.

        // TODO(xp): 111 test adding a node that is not non-voter.
        // TODO(xp): 111 test adding a node that is lagging.
        for new_node in members.difference(&self.core.membership.membership.get_ith_config(0).unwrap()) {
            match self.nodes.get(&new_node) {
                // Node is ready to join.
                Some(node) => {
                    if node.is_line_rate(&self.core.last_log_id, &self.core.config) {
                        continue;
                    }

                    if !blocking {
                        // Node has repl stream, but is not yet ready to join.
                        let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                            ChangeMembershipError::NonVoterIsLagging {
                                node_id: *new_node,
                                matched: node.matched,
                                distance: self.core.last_log_id.index.saturating_sub(node.matched.index),
                            },
                        )));
                        return;
                    }
                }

                // Node does not yet have a repl stream, spawn one.
                None => {
                    let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                        ChangeMembershipError::NonVoterNotFound { node_id: *new_node },
                    )));
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
        resp_tx: Option<RaftRespTx<ClientWriteResponse<R>, ClientWriteError>>,
    ) -> Result<(), RaftError> {
        let payload = ClientWriteRequest::<D>::new_config(mem.clone());
        let res = self.append_payload_to_log(payload.entry).await;

        // Caveat: membership must be updated before commit check is done with the new config.
        self.core.membership = EffectiveMembership {
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
                    if let Err(_e) = send_res {
                        tracing::error!("send response res error");
                    }
                }
                return Err(RaftError::RaftStorage(anyhow::anyhow!(err_str)));
            }
        };

        let cr_entry = ClientRequestEntry {
            entry: Arc::new(entry),
            tx: resp_tx,
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

        // Step down if needed.
        if !self.core.membership.membership.contains(&self.core.id) {
            tracing::debug!("raft node is stepping down");

            // TODO(xp): transfer leadership
            self.core.set_target_state(State::NonVoter);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            return;
        }

        let membership = &self.core.membership.membership;

        let all = membership.all_nodes();
        for (id, state) in self.nodes.iter_mut() {
            if all.contains(id) {
                continue;
            }

            tracing::info!(
                "set remove_after_commit for {} = {}, membership: {:?}",
                id,
                index,
                self.core.membership
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
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn try_remove_replication(&mut self, target: u64) -> bool {
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
