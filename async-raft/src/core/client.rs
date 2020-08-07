use std::sync::Arc;

use tokio::sync::oneshot;

use crate::{AppData, AppDataResponse, RaftNetwork, RaftStorage};
use crate::core::{LeaderState, State};
use crate::error::{ClientError, RaftError, RaftResult};
use crate::raft::{ClientRequest, ClientResponse, ClientResponseTx, Entry, EntryPayload};
use crate::replication::RaftEvent;

/// A wrapper around a ClientRequest which has been transformed into an Entry, along with its response channel.
pub(super) struct ClientRequestEntry<D: AppData, R: AppDataResponse> {
    /// The Arc'd entry of the ClientRequest.
    ///
    /// This value is Arc'd so that it may be sent across thread boundaries for replication
    /// without having to clone the data payload itself.
    pub entry: Arc<Entry<D>>,
    /// The response channel for the request.
    pub tx: ClientOrInternalResponseTx<D, R>,
}

impl<D: AppData, R: AppDataResponse> ClientRequestEntry<D, R> {
    /// Create a new instance from the raw components of a client request.
    pub(crate) fn from_entry<T: Into<ClientOrInternalResponseTx<D, R>>>(entry: Entry<D>, tx: T) -> Self {
        Self{entry: Arc::new(entry), tx: tx.into()}
    }
}

/// An enum type wrapping either a client response channel or an internal Raft response channel.
#[derive(derive_more::From)]
pub enum ClientOrInternalResponseTx<D: AppData, R: AppDataResponse> {
    Client(ClientResponseTx<D, R>),
    Internal(oneshot::Sender<Result<u64, RaftError>>),
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    /// Commit the initial entry which new leaders are obligated to create when first coming to power, per §8.
    #[tracing::instrument(level="trace", skip(self))]
    pub(super) async fn commit_initial_leader_entry(&mut self) -> RaftResult<()> {
        // If the cluster has just formed, and the current index is 0, then commit the current
        // config, else a blank payload.
        let req: ClientRequest<D> = if self.core.last_log_index == 0 {
            ClientRequest::new_config(self.core.membership.clone())
        } else {
            ClientRequest::new_blank_payload()
        };

        // Check to see if we have any config change logs newer than our commit index. If so, then
        // we need to drive the commitment of the config change to the cluster.
        let mut pending_config = None; // The inner bool represents `is_in_join_consensus`.
        if &self.core.last_log_index > &self.core.commit_index {
            let (stale_logs_start, stale_logs_stop) = (self.core.commit_index + 1, self.core.last_log_index + 1);
            pending_config = self.core.storage.get_log_entries(stale_logs_start, stale_logs_stop).await
                .map_err(|err| self.core.map_fatal_storage_error(err))?
                // Find the most recent config change.
                .iter().rev()
                .filter_map(|entry| match &entry.payload {
                    EntryPayload::ConfigChange(cfg) => Some(cfg.membership.is_in_joint_consensus()),
                    EntryPayload::SnapshotPointer(cfg) => Some(cfg.membership.is_in_joint_consensus()),
                    _ => None,
                })
                .nth(0);
        }

        // Commit the initial payload to the cluster.
        let (tx_payload_committed, rx_payload_committed) = oneshot::channel();
        let entry = self.append_payload_to_log(req.entry).await?;
        self.core.last_log_term = self.core.current_term; // This only ever needs to be updated once per term.
        let cr_entry = ClientRequestEntry::from_entry(entry, tx_payload_committed);
        self.replicate_client_request(cr_entry).await;
        self.core.report_metrics();

        // Setup any callbacks needed for responding to commitment of a pending config.
        if let Some(is_in_join_consensus) = pending_config {
            if is_in_join_consensus {
                self.joint_consensus_cb.push(rx_payload_committed); // Receiver for when the joint consensus is committed.
            } else {
                self.uniform_consensus_cb.push(rx_payload_committed); // Receiver for when the uniform consensus is committed.
            }
        }
        Ok(())
    }

    /// Handle client requests.
    #[tracing::instrument(level="trace", skip(self, rpc, tx))]
    pub(super) async fn handle_client_request(&mut self, rpc: ClientRequest<D>, tx: ClientResponseTx<D, R>) {
        let entry = match self.append_payload_to_log(rpc.entry).await {
            Ok(entry) => ClientRequestEntry::from_entry(entry, tx),
            Err(err) => {
                let _ = tx.send(Err(ClientError::RaftError(err)));
                return;
            }
        };
        self.replicate_client_request(entry).await;
    }

    /// Transform the given payload into an entry, assign an index and term, and append the entry to the log.
    #[tracing::instrument(level="trace", skip(self, payload))]
    pub(super) async fn append_payload_to_log(&mut self, payload: EntryPayload<D>) -> RaftResult<Entry<D>> {
        let entry = Entry{index: self.core.last_log_index + 1, term: self.core.current_term, payload};
        self.core.storage.append_entry_to_log(&entry).await.map_err(|err| self.core.map_fatal_storage_error(err))?;
        self.core.last_log_index = entry.index;
        Ok(entry)
    }

    /// Begin the process of replicating the given client request.
    ///
    /// NOTE WELL: this routine does not wait for the request to actually finish replication, it
    /// merely beings the process. Once the request is committed to the cluster, its response will
    /// be generated asynchronously.
    #[tracing::instrument(level="trace", skip(self, req))]
    pub(super) async fn replicate_client_request(&mut self, req: ClientRequestEntry<D, R>) {
        // Replicate the request if there are other cluster members. The client response will be
        // returned elsewhere after the entry has been committed to the cluster.
        let entry_arc = req.entry.clone();
        if !self.nodes.is_empty() {
            self.awaiting_committed.push(req);
            for node in self.nodes.values() {
                let _ = node.replstream.repltx.send(RaftEvent::Replicate{
                    entry: entry_arc.clone(),
                    commit_index: self.core.commit_index,
                });
            }
        } else {
            // Else, there are no voting nodes for replication, so the payload is now committed.
            self.core.commit_index = entry_arc.index;
            self.core.report_metrics();
            self.client_request_post_commit(req).await;
        }

        // Replicate to non-voters.
        if !self.non_voters.is_empty() {
            for node in self.non_voters.values() {
                let _ = node.state.replstream.repltx.send(RaftEvent::Replicate{
                    entry: entry_arc.clone(),
                    commit_index: self.core.commit_index,
                });
            }
        }
    }

    /// Handle the post-commit logic for a client request.
    #[tracing::instrument(level="trace", skip(self, req))]
    pub(super) async fn client_request_post_commit(&mut self, req: ClientRequestEntry<D, R>) {
        match req.tx {
            // If this is a client response channel, then it means that we are dealing with
            ClientOrInternalResponseTx::Client(tx) => match &req.entry.payload {
                EntryPayload::Normal(inner) => {
                    match self.apply_entry_to_state_machine(&req.entry.index, &inner.data).await {
                        Ok(data) => {
                            let _ = tx.send(Ok(ClientResponse{index: req.entry.index, data}));
                        }
                        Err(err) => {
                            let _ = tx.send(Err(ClientError::RaftError(RaftError::from(err))));
                        }
                    }
                }
                _ => {
                    // Why is this a bug, and why are we shutting down? This is because we can not easily
                    // encode these constraints in the type system, and client requests should be the only
                    // log entry types for which a `ClientOrInternalResponseTx::Client` type is used. This
                    // error should never be hit unless we've done a poor job in code review.
                    tracing::error!("critical error in Raft, this is a programming bug, please open an issue");
                    self.core.set_target_state(State::Shutdown);
                }
            }
            ClientOrInternalResponseTx::Internal(tx) => {
                self.core.last_applied = req.entry.index;
                self.core.report_metrics();
                let _ = tx.send(Ok(req.entry.index));
            }
        }

        // Trigger log compaction if needed.
        self.core.trigger_log_compaction_if_needed();
    }

    /// Apply the given log entry to the state machine.
    #[tracing::instrument(level="trace", skip(self, entry))]
    pub(super) async fn apply_entry_to_state_machine(&mut self, index: &u64, entry: &D) -> RaftResult<R> {
        // First, we just ensure that we apply any outstanding up to, but not including, the index
        // of the given entry. We need to be able to return the data response from applying this
        // entry to the state machine.
        //
        // Note that this would only ever happen if a node had unapplied logs from before becoming leader.
        let expected_next_index = self.core.last_applied + 1;
        if index != &expected_next_index {
            let entries = self.core.storage.get_log_entries(expected_next_index, *index).await.map_err(|err| self.core.map_fatal_storage_error(err))?;
            if let Some(entry) = entries.last() {
                self.core.last_applied = entry.index;
            }
            let data_entries: Vec<_> = entries.iter()
                .filter_map(|entry| match &entry.payload {
                    EntryPayload::Normal(inner) => Some((&entry.index, &inner.data)),
                    _ => None,
                })
                .collect();
            if !data_entries.is_empty() {
                self.core.storage.replicate_to_state_machine(&data_entries).await.map_err(|err| self.core.map_fatal_storage_error(err))?;
            }
        }

        // Apply this entry to the state machine and return its data response.
        let res = self.core.storage.apply_entry_to_state_machine(index, entry).await.map_err(|err| self.core.map_fatal_storage_error(err))?;
        self.core.last_applied = *index;
        self.core.report_metrics();
        Ok(res)
    }
}
