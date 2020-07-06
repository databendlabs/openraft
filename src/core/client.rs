use std::sync::Arc;

use tokio::sync::oneshot;

use crate::{AppData, AppDataResponse, AppError, RaftNetwork, RaftStorage};
use crate::core::LeaderState;
use crate::error::{ClientError, RaftError, RaftResult};
use crate::raft::TxClientResponse;
use crate::raft::{ClientRequest, ClientResponse, Entry, EntryPayload, ResponseMode};
use crate::replication::RaftEvent;

/// A wrapper around a ClientRequest which has been transformed into an Entry, along with its ResponseMode & channel.
pub(super) struct ClientRequestEntry<D: AppData, R: AppDataResponse, E: AppError> {
    /// The Arc'd entry of the ClientRequest.
    ///
    /// This value is Arc'd so that it may be sent across thread boundaries for replication
    /// without having to clone the data payload itself.
    pub entry: Arc<Entry<D>>,
    pub response_mode: ResponseMode,
    pub tx: TxClientResponse<D, R, E>,
}

impl<D: AppData, R: AppDataResponse, E: AppError> ClientRequestEntry<D, R, E> {
    /// Create a new instance from the raw components of a client request.
    pub(crate) fn from_entry(entry: Entry<D>, response_mode: ResponseMode, tx: TxClientResponse<D, R, E>) -> Self {
        Self{entry: Arc::new(entry), response_mode, tx}
    }
}

impl<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> LeaderState<'a, D, R, E, N, S> {
    /// Commit the initial entry which new leaders are obligated to create when first coming to power, per §8.
    pub(super) async fn commit_initial_leader_entry(&mut self) -> RaftResult<(), E> {
        // If the cluster has just formed, and the current index is 0, then commit the current
        // config, else a blank payload.
        let req: ClientRequest<D> = if self.core.last_log_index == 0 {
            ClientRequest::new_config(self.core.membership.clone())
        } else {
            ClientRequest::new_blank_payload()
        };

        // Check to see if we have any config change logs newer than our commit index. If so, then
        // we need to drive the committment of the config change to the cluster.
        let mut pending_config = None; // The inner bool represents `is_in_join_consensus`.
        if &self.core.last_log_index > &self.core.commit_index {
            let (stale_logs_start, stale_logs_stop) = (self.core.commit_index + 1, self.core.last_log_index + 1);
            pending_config = self.core.storage.get_log_entries(stale_logs_start, stale_logs_stop).await
                .map_err(|err| self.core.map_fatal_storage_result(err))?
                // Find the most recent config change.
                .iter().rev()
                .filter_map(|entry| match &entry.payload {
                    EntryPayload::ConfigChange(cfg) => Some(cfg.membership.is_in_joint_consensus),
                    _ => None,
                })
                .nth(0);
        }

        // Commit the initial payload to the cluster.
        let (tx_payload_committed, rx_payload_committed) = oneshot::channel();
        let entry = self.append_payload_to_log(req.entry).await?;
        self.core.last_log_term = self.core.current_term; // This only ever needs to be updated once per term.
        let cr_entry = ClientRequestEntry::from_entry(entry, req.response_mode, tx_payload_committed);
        self.replicate_client_request(cr_entry).await;

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
    pub(super) async fn handle_client_request(&mut self, rpc: ClientRequest<D>, tx: TxClientResponse<D, R, E>) {
        let entry = match self.append_payload_to_log(rpc.entry).await {
            Ok(entry) => ClientRequestEntry::from_entry(entry, rpc.response_mode, tx),
            Err(err) => {
                let _ = tx.send(Err(ClientError::RaftError(err)));
                return;
            }
        };
        self.replicate_client_request(entry).await;
    }

    /// Transform the given payload into an entry, assign an index and term, and append the entry to the log.
    pub(super) async fn append_payload_to_log(&mut self, payload: EntryPayload<D>) -> RaftResult<Entry<D>, E> {
        let entry = Entry{index: self.core.last_log_index + 1, term: self.core.current_term, payload};
        self.core.storage.append_entry_to_log(&entry).await.map_err(|err| self.core.map_fatal_storage_result(err))?;
        self.core.last_log_index = entry.index;
        Ok(entry)
    }

    /// Begin the process of replicating the given client request.
    ///
    /// NOTE WELL: this routine does not wait for the request to actually finish replication, it
    /// merely beings the process. Once the request is committed to the cluster, its response will
    /// be generated asynchronously.
    pub(super) async fn replicate_client_request(&mut self, req: ClientRequestEntry<D, R, E>) {
        // Replicate the request if there are other cluster members. The client response will be
        // returned elsewhere after the entry has been committed to the cluster.
        if self.nodes.len() > 0 {
            let entry_arc = req.entry.clone();
            self.awaiting_committed.push(req);
            for node in self.nodes.values() {
                let _ = node.replstream.repltx.send(RaftEvent::Replicate{
                    entry: entry_arc.clone(),
                    commit_index: self.core.commit_index,
                });
            }
            return;
        }

        // Else, there are no nodes for replication, so the payload is now committed.
        self.core.commit_index = req.entry.index;
        self.client_request_post_commit(req).await;
    }

    /// Handle the post-commit logic for a client request.
    pub(super) async fn client_request_post_commit(&mut self, req: ClientRequestEntry<D, R, E>) {
        // Respond to the user per the requested client response mode.
        match &req.response_mode {
            ResponseMode::Committed => {
                let _ = req.tx.send(Ok(ClientResponse::Committed{index: req.entry.index}));
                let _ = self.core.apply_entry_to_state_machine(&req.entry).await;
            }
            ResponseMode::Applied => {
                match self.core.apply_entry_to_state_machine(&req.entry).await {
                    Ok(data) => {
                        let _ = req.tx.send(Ok(ClientResponse::Applied{index: req.entry.index, data}));
                    }
                    Err(err) => {
                        let _ = req.tx.send(Err(ClientError::RaftError(RaftError::from(err))));
                    }
                }
            }
        }

        // Trigger log compaction if needed.
        self.core.trigger_log_compaction_if_needed();
    }
}
