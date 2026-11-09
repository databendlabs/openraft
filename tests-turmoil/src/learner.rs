//! Application-owned learner API shared by scenarios and the membership client.

use openraft::errors::ClientWriteError;
use openraft::errors::RaftError;
use openraft::raft::ClientWriteResponse;

use crate::typ::Node;
use crate::typ::NodeId;
use crate::typ::Raft;
use crate::typ::TypeConfig;

/// Adds a learner using the configured edit, optionally waiting for replication.
#[openraft::macros::since(version = "0.10.0")]
pub async fn add_learner(
    raft: &Raft,
    id: NodeId,
    node: Node,
    blocking: bool,
) -> Result<ClientWriteResponse<TypeConfig>, RaftError<TypeConfig, ClientWriteError<TypeConfig>>> {
    #[cfg(not(feature = "threshold-membership"))]
    {
        raft.add_learner(id, node, blocking).await
    }
    #[cfg(feature = "threshold-membership")]
    {
        use std::collections::BTreeMap;

        use openraft::async_runtime::WatchReceiver;
        use openraft_membership::ThresholdChange;

        let change = ThresholdChange {
            add_nodes: BTreeMap::from([(id, node)]),
            ..Default::default()
        };
        let response = raft.change_membership(change, true).await?;
        if !blocking || id == raft.metrics().borrow_watched().id {
            return Ok(response);
        }

        let threshold = raft.config().replication_lag_threshold;
        let wait_result = raft.wait(None).replication(id, Some(response.log_id.index()), threshold).await;
        // Preserve the default API's completion on removal, leadership loss, or shutdown.
        if let Err(error) = wait_result {
            println!("MEMBERSHIP-CLIENT: learner {id} replication wait ended: {error}");
        }
        Ok(response)
    }
}
