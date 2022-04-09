use futures::future::TryFutureExt;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use maplit::btreeset;
use tokio::time::timeout;
use tokio::time::Duration;
use tracing::Instrument;

use crate::core::apply_to_state_machine;
use crate::core::LeaderState;
use crate::core::State;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::Timeout;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ClientWriteRequest;
use crate::raft::ClientWriteResponse;
use crate::raft::RaftRespTx;
use crate::replication::RaftEvent;
use crate::Entry;
use crate::EntryPayload;
use crate::LogId;
use crate::MessageSummary;
use crate::RPCTypes;
use crate::RaftNetwork;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    /// Commit the initial entry which new leaders are obligated to create when first coming to power, per §8.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) async fn commit_initial_leader_entry(&mut self) -> Result<(), StorageError<C::NodeId>> {
        let entry = self.core.append_payload_to_log(EntryPayload::Blank).await?;

        self.core.metrics_flags.set_data_changed();

        self.replicate_client_request(entry.log_id, None).await?;

        Ok(())
    }

    /// Handle `is_leader` requests.
    ///
    /// Spawn requests to all members of the cluster, include members being added in joint
    /// consensus. Each request will have a timeout, and we respond once we have a majority
    /// agreement from each config group. Most of the time, we will have a single uniform
    /// config group.
    ///
    /// From the spec (§8):
    /// Second, a leader must check whether it has been deposed before processing a read-only
    /// request (its information may be stale if a more recent leader has been elected). Raft
    /// handles this by having the leader exchange heartbeat messages with a majority of the
    /// cluster before responding to read-only requests.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(super) async fn handle_check_is_leader_request(&mut self, tx: RaftRespTx<(), CheckIsLeaderError<C::NodeId>>) {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let mem = &self.core.effective_membership.membership;
        let mut granted = btreeset! {self.core.id};

        if mem.is_majority(&granted) {
            let _ = tx.send(Ok(()));
            return;
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();
        let membership = &self.core.effective_membership.membership;

        for (target, node) in self.nodes.iter() {
            if !membership.is_member(target) {
                continue;
            }

            let rpc = AppendEntriesRequest {
                vote: self.core.vote,
                prev_log_id: node.matched,
                entries: vec![],
                leader_commit: self.core.committed,
            };

            let my_id = self.core.id;
            let target = *target;
            let target_node = self.core.effective_membership.get_node(&target).cloned();
            let mut network = self.core.network.connect(target, target_node.as_ref()).await;

            let ttl = Duration::from_millis(self.core.config.heartbeat_interval);

            let task = tokio::spawn(
                async move {
                    let outer_res = timeout(ttl, network.send_append_entries(rpc)).await;
                    match outer_res {
                        Ok(append_res) => match append_res {
                            Ok(x) => Ok((target, x)),
                            Err(err) => Err((target, err)),
                        },
                        Err(_timeout) => {
                            let timeout_err = Timeout {
                                action: RPCTypes::AppendEntries,
                                id: my_id,
                                target,
                                timeout: ttl,
                            };

                            Err((target, RPCError::Timeout(timeout_err)))
                        }
                    }
                }
                // TODO(xp): add target to span
                .instrument(tracing::debug_span!("SPAWN_append_entries")),
            )
            .map_err(move |err| (target, err));

            pending.push(task);
        }

        // Handle responses as they return.
        while let Some(res) = pending.next().await {
            let (target, data) = match res {
                Ok(Ok(res)) => res,
                Ok(Err((target, err))) => {
                    tracing::error!(target=display(target), error=%err, "timeout while confirming leadership for read request");
                    continue;
                }
                Err((target, err)) => {
                    tracing::error!(target = display(target), "{}", err);
                    continue;
                }
            };

            // If we receive a response with a greater term, then revert to follower and abort this request.
            if let AppendEntriesResponse::HigherVote(vote) = data {
                assert!(vote > self.core.vote);
                self.core.vote = vote;
                // TODO(xp): deal with storage error
                self.core.save_vote().await.unwrap();
                // TODO(xp): if receives error about a higher term, it should stop at once?
                self.core.set_target_state(State::Follower);
            }

            granted.insert(target);

            let mem = &self.core.effective_membership.membership;
            if mem.is_majority(&granted) {
                let _ = tx.send(Ok(()));
                return;
            }
        }

        // If we've hit this location, then we've failed to gather needed confirmations due to
        // request failures.

        let _ = tx.send(Err(QuorumNotEnough {
            cluster: self.core.effective_membership.membership.summary(),
            got: granted,
        }
        .into()));
    }

    /// Handle client write requests.
    #[tracing::instrument(level = "trace", skip(self, tx), fields(rpc=%rpc.summary()))]
    pub(super) async fn handle_client_write_request(
        &mut self,
        rpc: ClientWriteRequest<C>,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        let entry = self.core.append_payload_to_log(rpc.payload).await?;

        self.core.metrics_flags.set_data_changed();

        self.replicate_client_request(entry.log_id, Some(tx)).await?;
        Ok(())
    }

    /// Begin the process of replicating the given client request.
    ///
    /// NOTE WELL: this routine does not wait for the request to actually finish replication, it
    /// merely beings the process. Once the request is committed to the cluster, its response will
    /// be generated asynchronously.
    #[tracing::instrument(level = "debug", skip(self, resp_tx), fields(log_id=%log_id))]
    pub(super) async fn replicate_client_request(
        &mut self,
        log_id: LogId<C::NodeId>,
        resp_tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Install callback in which the entry will be applied to state machine.
        if let Some(tx) = resp_tx {
            self.client_resp_channels.insert(log_id.index, tx);
        }

        let quorum_granted = self.core.effective_membership.membership.is_majority(&btreeset! {self.core.id});

        if quorum_granted {
            assert!(self.core.committed < Some(log_id));

            self.core.committed = Some(log_id);
            tracing::debug!(?self.core.committed, "update committed, no need to replicate");

            self.core.metrics_flags.set_data_changed();
            self.client_request_post_commit(log_id.index).await?;
        }

        for node in self.nodes.values() {
            let _ = node.repl_stream.repl_tx.send((
                RaftEvent::Replicate {
                    appended: log_id,
                    committed: self.core.committed,
                },
                tracing::debug_span!("CH"),
            ));
        }

        Ok(())
    }

    /// Handle the post-commit logic for a client request.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn client_request_post_commit(&mut self, log_index: u64) -> Result<(), StorageError<C::NodeId>> {
        let entries = self.core.storage.get_log_entries(log_index..=log_index).await?;
        let entry = &entries[0];

        let tx = self.client_resp_channels.remove(&log_index);

        let apply_res = self.apply_entry_to_state_machine(entry).await?;

        self.send_response(entry, apply_res, tx).await;

        // Trigger log compaction if needed.
        self.core.trigger_log_compaction_if_needed(false).await;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, entry, resp, tx), fields(entry=%entry.summary()))]
    pub(super) async fn send_response(
        &mut self,
        entry: &Entry<C>,
        resp: C::R,
        tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,
    ) {
        let tx = match tx {
            None => return,
            Some(x) => x,
        };

        let membership = if let EntryPayload::Membership(ref c) = entry.payload {
            Some(c.clone())
        } else {
            None
        };

        let res = Ok(ClientWriteResponse {
            log_id: entry.log_id,
            data: resp,
            membership,
        });

        let send_res = tx.send(res);
        tracing::debug!(
            "send client response through tx, send_res is error: {}",
            send_res.is_err()
        );
    }

    pub fn handle_special_log(&mut self, entry: &Entry<C>) {
        match &entry.payload {
            EntryPayload::Membership(ref m) => {
                if m.is_in_joint_consensus() {
                    // nothing to do
                } else {
                    self.handle_uniform_consensus_committed(&entry.log_id);
                }
            }
            EntryPayload::Blank => {}
            EntryPayload::Normal(_) => {}
        }
    }

    /// Apply the given log entry to the state machine.
    #[tracing::instrument(level = "debug", skip(self, entry))]
    pub(super) async fn apply_entry_to_state_machine(
        &mut self,
        entry: &Entry<C>,
    ) -> Result<C::R, StorageError<C::NodeId>> {
        self.handle_special_log(entry);

        // First, we just ensure that we apply any outstanding up to, but not including, the index
        // of the given entry. We need to be able to return the data response from applying this
        // entry to the state machine.
        //
        // Note that this would only ever happen if a node had unapplied logs from before becoming leader.

        let log_id = &entry.log_id;
        let index = log_id.index;

        let expected_next_index = match self.core.last_applied {
            None => 0,
            Some(log_id) => log_id.index + 1,
        };

        if index != expected_next_index {
            let entries = self.core.storage.get_log_entries(expected_next_index..index).await?;

            if let Some(entry) = entries.last() {
                self.core.last_applied = Some(entry.log_id);
            }

            let data_entries: Vec<_> = entries.iter().collect();
            if !data_entries.is_empty() {
                apply_to_state_machine(
                    &mut self.core.storage,
                    &data_entries,
                    self.core.config.max_applied_log_to_keep,
                )
                .await?;
            }
        }

        // Apply this entry to the state machine and return its data response.
        let apply_res = apply_to_state_machine(
            &mut self.core.storage,
            &[entry],
            self.core.config.max_applied_log_to_keep,
        )
        .await?;

        // TODO(xp): deal with partial apply.
        self.core.last_applied = Some(*log_id);
        self.core.metrics_flags.set_data_changed();

        // TODO(xp) merge this function to replication_to_state_machine?

        Ok(apply_res.into_iter().next().unwrap())
    }
}
