use std::sync::Arc;

use toy_rpc::macros::export_impl;

use crate::app::App;
use crate::typ::*;

/// Raft protocol service.
pub struct Raft {
    app: Arc<App>,
}

#[export_impl]
impl Raft {
    pub fn new(app: Arc<App>) -> Self {
        Self { app }
    }

    #[export_method]
    pub async fn vote(&self, vote: VoteRequest) -> Result<VoteResponse, toy_rpc::Error> {
        self.app.raft.vote(vote).await.map_err(|e| toy_rpc::Error::Internal(Box::new(e)))
    }

    #[export_method]
    pub async fn append(&self, req: AppendEntriesRequest) -> Result<AppendEntriesResponse, toy_rpc::Error> {
        tracing::debug!("handle append");
        self.app.raft.append_entries(req).await.map_err(|e| toy_rpc::Error::Internal(Box::new(e)))
    }

    #[export_method]
    pub async fn snapshot(&self, req: InstallSnapshotRequest) -> Result<InstallSnapshotResponse, toy_rpc::Error> {
        self.app.raft.install_snapshot(req).await.map_err(|e| toy_rpc::Error::Internal(Box::new(e)))
    }
}
