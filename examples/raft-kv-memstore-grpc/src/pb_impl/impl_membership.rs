use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Membership;

use crate::pb;
use crate::TypeConfig;

impl From<pb::Membership> for Membership<TypeConfig> {
    fn from(value: pb::Membership) -> Self {
        let mut configs = vec![];
        for c in value.configs {
            let config: BTreeSet<u64> = c.node_ids.keys().copied().collect();
            configs.push(config);
        }
        let nodes = value.nodes;
        // TODO: do not unwrap()
        Membership::new(configs, nodes).unwrap()
    }
}

impl From<Membership<TypeConfig>> for pb::Membership {
    fn from(value: Membership<TypeConfig>) -> Self {
        let mut configs = vec![];
        for c in value.get_joint_config() {
            let mut node_ids = BTreeMap::new();
            for nid in c.iter() {
                node_ids.insert(*nid, ());
            }
            configs.push(pb::NodeIdSet { node_ids });
        }
        let nodes = value.nodes().map(|(nid, n)| (*nid, n.clone())).collect();
        pb::Membership { configs, nodes }
    }
}
