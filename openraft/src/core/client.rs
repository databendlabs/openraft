use std::sync::Arc;

use anyhow::anyhow;
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
use crate::error::ClientReadError;
use crate::error::ClientWriteError;
use crate::error::QuorumNotEnough;
use crate::raft::AppendEntriesRequest;
use crate::raft::ClientWriteRequest;
use crate::raft::ClientWriteResponse;
use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::raft::RaftRespTx;
use crate::replication::RaftEvent;
use crate::AppData;
use crate::AppDataResponse;
use crate::MessageSummary;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::StorageError;

/// A wrapper around a ClientRequest which has been transformed into an Entry, along with its response channel.
pub(super) struct ClientRequestEntry<D: AppData, R: AppDataResponse> {
    /// The Arc'd entry of the ClientRequest.
    ///
    /// This value is Arc'd so that it may be sent across thread boundaries for replication
    /// without having to clone the data payload itself.
    pub entry: Arc<Entry<D>>,

    /// The response channel for the request.
    pub tx: Option<RaftRespTx<ClientWriteResponse<R>, ClientWriteError>>,
}

impl<D: AppData, R: AppDataResponse> MessageSummary for ClientRequestEntry<D, R> {
    fn summary(&self) -> String {
        format!("entry:{}", self.entry.summary())
    }
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    /// Commit the initial entry which new leaders are obligated to create when first coming to power, per §8.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) async fn commit_initial_leader_entry(&mut self) -> Result<(), StorageError> {
        let entry = self.core.append_payload_to_log(EntryPayload::Blank).await?;

        self.leader_report_metrics();

        let cr_entry = ClientRequestEntry {
            entry: Arc::new(entry),
            tx: None,
        };

        self.replicate_client_request(cr_entry).await?;

        Ok(())
    }

    /// Handle client read requests.
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
    pub(super) async fn handle_client_read_request(&mut self, tx: RaftRespTx<(), ClientReadError>) {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let mem = &self.core.effective_membership.membership;
        let mut granted = btreeset! {self.core.id};

        if mem.is_majority(&granted) {
            let _ = tx.send(Ok(()));
            return;
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();
        let all_members = self.core.effective_membership.membership.all_nodes();

        for (id, node) in self.nodes.iter() {
            if !all_members.contains(id) {
                continue;
            }

            let rpc = AppendEntriesRequest {
                term: self.core.current_term,
                leader_id: self.core.id,
                prev_log_id: node.matched,
                entries: vec![],
                leader_commit: self.core.committed,
            };

            let target = *id;
            let network = self.core.network.clone();

            let ttl = Duration::from_millis(self.core.config.heartbeat_interval);

            let task = tokio::spawn(
                async move {
                    match timeout(ttl, network.send_append_entries(target, rpc)).await {
                        Ok(Ok(data)) => Ok((target, data)),
                        Ok(Err(err)) => Err((target, err)),
                        Err(_timeout) => Err((target, anyhow!("timeout waiting for leadership confirmation"))),
                    }
                }
                .instrument(tracing::debug_span!("spawn")),
            )
            .map_err(move |err| (*id, err));
            pending.push(task);
        }

        // Handle responses as they return.
        while let Some(res) = pending.next().await {
            let (target, data) = match res {
                Ok(Ok(res)) => res,
                Ok(Err((target, err))) => {
                    tracing::error!(target, error=%err, "timeout while confirming leadership for read request");
                    continue;
                }
                Err((target, err)) => {
                    tracing::error!(target, "{}", err);
                    continue;
                }
            };

            // If we receive a response with a greater term, then revert to follower and abort this request.
            if data.term != self.core.current_term {
                self.core.update_current_term(data.term, None);
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
        rpc: ClientWriteRequest<D>,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    ) -> Result<(), StorageError> {
        let entry = self.core.append_payload_to_log(rpc.payload).await?;
        let entry = ClientRequestEntry {
            entry: Arc::new(entry),
            tx: Some(tx),
        };

        self.leader_report_metrics();

        self.replicate_client_request(entry).await?;
        Ok(())
    }

    /// Begin the process of replicating the given client request.
    ///
    /// NOTE WELL: this routine does not wait for the request to actually finish replication, it
    /// merely beings the process. Once the request is committed to the cluster, its response will
    /// be generated asynchronously.
    #[tracing::instrument(level = "debug", skip(self, req), fields(req=%req.summary()))]
    pub(super) async fn replicate_client_request(&mut self, req: ClientRequestEntry<D, R>) -> Result<(), StorageError> {
        // Replicate the request if there are other cluster members. The client response will be
        // returned elsewhere after the entry has been committed to the cluster.
        let entry_arc = req.entry.clone();

        // TODO(xp): calculate nodes set that need to replicate to, when updating membership
        // TODO(xp): Or add to-learner replication into self.nodes.

        let all_members = self.core.effective_membership.membership.all_nodes();

        let nodes = self.nodes.keys().collect::<Vec<_>>();
        tracing::debug!(?nodes, ?all_members, "replicate_client_request");

        // Except the leader itself, there are other nodes that need to replicate log to.
        let await_quorum = all_members.len() > 1;

        if await_quorum {
            self.awaiting_committed.push(req);
        } else {
            // Else, there are no voting nodes for replication, so the payload is now committed.
            self.core.committed = Some(entry_arc.log_id);
            tracing::debug!(?self.core.committed, "update committed, no need to replicate");

            self.leader_report_metrics();
            self.client_request_post_commit(req).await?;
        }

        for node in self.nodes.values() {
            let _ = node.repl_stream.repl_tx.send((
                RaftEvent::Replicate {
                    appended: entry_arc.log_id,
                    committed: self.core.committed,
                },
                tracing::debug_span!("CH"),
            ));
        }

        Ok(())
    }

    /// Handle the post-commit logic for a client request.
    #[tracing::instrument(level = "debug", skip(self, req))]
    pub(super) async fn client_request_post_commit(
        &mut self,
        req: ClientRequestEntry<D, R>,
    ) -> Result<(), StorageError> {
        let entry = &req.entry;

        let apply_res = self.apply_entry_to_state_machine(entry).await?;

        self.send_response(entry, apply_res, req.tx).await;

        // Trigger log compaction if needed.
        self.core.trigger_log_compaction_if_needed(false);
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, entry, resp, tx), fields(entry=%entry.summary()))]
    pub(super) async fn send_response(
        &mut self,
        entry: &Entry<D>,
        resp: R,
        tx: Option<RaftRespTx<ClientWriteResponse<R>, ClientWriteError>>,
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

    pub fn handle_special_log(&mut self, entry: &Entry<D>) {
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
    pub(super) async fn apply_entry_to_state_machine(&mut self, entry: &Entry<D>) -> Result<R, StorageError> {
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
                    self.core.storage.clone(),
                    &data_entries,
                    self.core.config.max_applied_log_to_keep,
                )
                .await?;
            }
        }

        // Apply this entry to the state machine and return its data response.
        let apply_res = apply_to_state_machine(
            self.core.storage.clone(),
            &[entry],
            self.core.config.max_applied_log_to_keep,
        )
        .await?;

        // TODO(xp): deal with partial apply.
        self.core.last_applied = Some(*log_id);
        self.leader_report_metrics();

        // TODO(xp) merge this function to replication_to_state_machine?

        Ok(apply_res.into_iter().next().unwrap())
    }
}
