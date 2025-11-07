use std::future::Future;

use async_recursion::async_recursion;
use openraft::error::ReplicationClosed;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::RPCOption;
use openraft::BasicNode;
use openraft::OptionalSend;
use openraft::RaftNetworkFactory;

use crate::router::Router;
use crate::typ::*;
use crate::NodeId;
use crate::TypeConfig;

pub struct Connection {
    router: Router,
    target: NodeId,
}

impl RaftNetworkFactory<TypeConfig> for Router {
    type Network = Connection;

    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Self::Network {
        Connection {
            router: self.clone(),
            target,
        }
    }
}

impl Connection {
    /// Helper method to send append_entries with automatic retry and chunking on payload-too-large
    /// errors.
    #[async_recursion]
    async fn send_append_entries_with_retry(
        &mut self,
        req: AppendEntriesRequest,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError> {
        // Try sending the full request first
        let resp = self.router.send(self.target, "/raft/append", &req).await;

        match resp {
            Ok(resp) => Ok(resp),
            Err(e) => {
                // Check if this is a payload-too-large error
                // In a real application, we would check for specific HTTP errors like 413,
                // gRPC RESOURCE_EXHAUSTED, or custom transport errors
                let error_msg = format!("{:?}", e);
                if error_msg.contains("payload too large") || error_msg.contains("too large") {
                    tracing::warn!("Payload too large, splitting {} entries into chunks", req.entries.len());

                    // Split entries in half and retry
                    if req.entries.is_empty() {
                        return Err(RPCError::Unreachable(e));
                    }

                    let mid = req.entries.len() / 2;
                    if mid == 0 {
                        return Err(RPCError::Unreachable(e));
                    }

                    // Send first half
                    let prev_log_id_for_second = req.entries.get(mid - 1).map(|e| e.log_id);
                    let mut first_req = req.clone();
                    first_req.entries.truncate(mid);
                    let first_resp = self.send_append_entries_with_retry(first_req, option.clone()).await?;

                    match first_resp {
                        AppendEntriesResponse::Success | AppendEntriesResponse::PartialSuccess(_) => {
                            // First chunk succeeded, send second half
                            let mut second_req = req;
                            second_req.entries.drain(..mid);
                            second_req.prev_log_id = prev_log_id_for_second;

                            self.send_append_entries_with_retry(second_req, option).await
                        }
                        _ => Ok(first_resp),
                    }
                } else {
                    Err(RPCError::Unreachable(e))
                }
            }
        }
    }
}

impl RaftNetworkV2<TypeConfig> for Connection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError> {
        self.send_append_entries_with_retry(req, option).await
    }

    /// A real application should replace this method with customized implementation.
    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError> {
        let resp = self.router.send(self.target, "/raft/snapshot", &(vote, snapshot.meta, snapshot.snapshot)).await?;
        Ok(resp)
    }

    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let resp = self.router.send(self.target, "/raft/vote", &req).await?;
        Ok(resp)
    }
}
