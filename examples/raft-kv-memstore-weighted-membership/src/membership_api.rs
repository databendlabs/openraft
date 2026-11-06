//! The membership routes of this example. They take the weight of each voter, which the voter-set
//! routes of `app-http` cannot carry.

use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::NodeInfo;
use openraft::errors::ClientWriteError;
use openraft::errors::InitializeError;
use openraft::errors::decompose::DecomposeResult;
use openraft::raft::ClientWriteResponse;
use openraft::type_config::TypeConfigExt;
use openraft_membership::WeightedChange;
use openraft_membership::WeightedMembership;
use serde::Deserialize;
use serde::Serialize;

use crate::NodeId;
use crate::TypeConfig;
use crate::app::App;

/// The body of `/init`: the first membership, as the weight of each voter and the write weight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitRequest {
    /// The voters and their weights. The read weight is `total_weight - write_weight + 1`.
    pub weights: BTreeMap<NodeId, u64>,

    /// The weight that the voters storing an entry must reach together to commit it.
    pub write_weight: u64,

    /// The addresses of the other nodes of the first membership. The node that serves `/init` adds
    /// its own.
    #[serde(default)]
    pub nodes: BTreeMap<NodeId, NodeInfo>,
}

/// The body of `/change-membership`: the new weights, and optionally a new write weight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeMembershipRequest {
    /// The new voters and their weights. A new voter must already be a learner.
    pub weights: BTreeMap<NodeId, u64>,

    /// The new write weight; `None` keeps the current one.
    #[serde(default)]
    pub write_weight: Option<u64>,
}

/// Initializes the cluster with the `WeightedMembership` that `req` describes.
///
/// A weight of 0, or a write weight outside `1..=total_weight`, fails with
/// `InitializeError::InvalidMembership`.
pub async fn init(app: Arc<App>, req: InitRequest) -> Result<(), InitializeError<TypeConfig>> {
    let mut nodes = req.nodes;
    let this_node = NodeInfo::new(app.raft_addr.clone(), app.api_addr.clone());
    nodes.entry(app.id).or_insert(this_node);

    let membership = WeightedMembership::new(req.weights, req.write_weight, nodes);
    let membership = membership.map_err(|e| e.map_user_defined(|e| TypeConfig::err_from_error(&e)))?;

    app.raft.initialize(membership).await.decompose().unwrap()
}

/// Sets the voters and their weights with a `WeightedChange`, and the write weight if `req` gives
/// one.
///
/// A change of the weights moves through a joint config, so OpenRaft appends two log entries, and
/// the response is the one of the second.
pub async fn change_membership(
    app: Arc<App>,
    req: ChangeMembershipRequest,
) -> Result<ClientWriteResponse<TypeConfig>, ClientWriteError<TypeConfig>> {
    let mut change = WeightedChange::weights(req.weights);
    if let Some(write_weight) = req.write_weight {
        change = change.with_write_weight(write_weight);
    }

    app.raft.change_membership(change, false).await.decompose().unwrap()
}
