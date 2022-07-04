use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;

use super::NodeId;

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset of a majority of
/// every config.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// Multi configs.
    pub(crate) configs: Vec<BTreeSet<NodeId>>,

    /// Cache of all node ids.
    pub(crate) all_nodes: BTreeSet<NodeId>,
}
