use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;

use crate::compat::Upgrade;

/// v0.7 compatible Membership.
///
/// To load from either v0.7 or the latest format data and upgrade it to the latest type:
/// ```ignore
/// let x:openraft::Membership = serde_json::from_slice::<compat07::Membership>(&serialized_bytes)?.upgrade()
/// ```
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Membership {
    pub configs: Vec<BTreeSet<u64>>,
    pub nodes: Option<BTreeMap<u64, crate::EmptyNode>>,
    pub all_nodes: Option<BTreeSet<u64>>,
}

impl Upgrade<crate::Membership<u64, crate::EmptyNode>> for or07::Membership {
    fn upgrade(self) -> crate::Membership<u64, crate::EmptyNode> {
        let configs = self.get_configs().clone();
        let nodes = self.all_nodes().iter().map(|nid| (*nid, crate::EmptyNode::new())).collect::<BTreeMap<_, _>>();
        crate::Membership::new(configs, nodes)
    }
}

impl Upgrade<crate::Membership<u64, crate::EmptyNode>> for Membership {
    fn upgrade(self) -> crate::Membership<u64, crate::EmptyNode> {
        if let Some(ns) = self.nodes {
            crate::Membership::new(self.configs, ns)
        } else {
            crate::Membership::new(self.configs, self.all_nodes.unwrap())
        }
    }
}
