//! Request and response messages of the v1 chunk-based snapshot RPC.

mod install_snapshot_request;
mod install_snapshot_response;

pub use install_snapshot_request::InstallSnapshotRequest;
pub use install_snapshot_response::InstallSnapshotResponse;
