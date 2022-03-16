use core::cmp::Ord;
use core::option::Option;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use serde::Deserialize;
use serde::Serialize;

use crate::error::MissingNodeInfo;
use crate::membership::quorum;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftTypeConfig;

/// Convert other types into the internal data structure for node infos
pub trait IntoOptionNodes<NID: NodeId> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>>;
}

impl<NID: NodeId> IntoOptionNodes<NID> for () {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        btreemap! {}
    }
}

impl<NID: NodeId> IntoOptionNodes<NID> for BTreeSet<NID> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        self.into_iter().map(|node_id| (node_id, None)).collect()
    }
}

impl<NID: NodeId> IntoOptionNodes<NID> for BTreeMap<NID, Node> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        self.into_iter().map(|(node_id, n)| (node_id, Some(n))).collect()
    }
}

impl<NID: NodeId> IntoOptionNodes<NID> for BTreeMap<NID, Option<Node>> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        self
    }
}

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset of a majority of
/// every config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership<C: RaftTypeConfig> {
    /// Multi configs of members.
    ///
    /// AKA a joint config in original raft paper.
    configs: Vec<BTreeSet<C::NodeId>>,

    /// Additional info of all nodes, e.g., the connecting host and port.
    ///
    /// A node-id key that is in `nodes` but is not in `configs` is a **learner**.
    /// The values in this map must all be `Some` or `None`.
    nodes: BTreeMap<C::NodeId, Option<Node>>,
}

impl<C: RaftTypeConfig> MessageSummary for Membership<C> {
    fn summary(&self) -> String {
        let mut res = vec!["members:[".to_string()];
        for (i, c) in self.configs.iter().enumerate() {
            if i > 0 {
                res.push(",".to_string());
            }

            res.push("{".to_string());
            for (i, node_id) in c.iter().enumerate() {
                if i > 0 {
                    res.push(",".to_string());
                }
                res.push(format!("{}", node_id));

                if let Some(n) = self.get_node(node_id) {
                    res.push(format!(":{{{}}}", n));
                }
            }
            res.push("}".to_string());
        }
        res.push("]".to_string());

        let all_node_ids = self.nodes.keys().cloned().collect::<BTreeSet<_>>();
        let members = self.build_member_ids();

        res.push(",learners:[".to_string());
        for (learner_cnt, learner_id) in all_node_ids.difference(&members).enumerate() {
            if learner_cnt > 0 {
                res.push(",".to_string());
            }

            res.push(format!("{}", learner_id));

            if let Some(n) = self.get_node(learner_id) {
                res.push(format!(":{{{}}}", n));
            }
        }
        res.push("]".to_string());
        res.join("")
    }
}

impl<C: RaftTypeConfig> Membership<C> {
    /// Create a new Membership of multiple configs(joint) and optionally a set of learner node ids.
    ///
    /// A node id that is in `node_ids` but is not in `configs` is a **learner**.
    pub fn new(configs: Vec<BTreeSet<C::NodeId>>, node_ids: Option<BTreeSet<C::NodeId>>) -> Self {
        let all_members = Self::build_all_member_ids(&configs);

        let nodes = match node_ids {
            None => {
                btreemap! {}
            }
            Some(x) => x.into_option_nodes(),
        };

        let nodes = Self::extend_nodes(nodes, &all_members.into_option_nodes());

        Membership { configs, nodes }
    }

    /// Create a new Membership of multiple configs and optional node infos.
    ///
    /// The node infos `nodes` can be:
    /// - a simple `()`, if there are no non-member nodes and no node infos are provided,
    /// - `BTreeSet<NodeId>` to specify additional learner node ids without node infos provided,
    /// - `BTreeMap<NodeId, Node>` or `BTreeMap<NodeId, Option<Node>>` to specify learner nodes with node infos. Node
    ///   ids not in `configs` are learner node ids. In this case, every node id in `configs` has to present in `nodes`
    ///   or an error will be returned.
    pub(crate) fn with_nodes<T>(configs: Vec<BTreeSet<C::NodeId>>, nodes: T) -> Result<Self, MissingNodeInfo<C>>
    where T: IntoOptionNodes<C::NodeId> {
        let all_members = Self::build_all_member_ids(&configs);

        let nodes = nodes.into_option_nodes();

        for node_id in all_members.iter() {
            if !nodes.contains_key(node_id) {
                return Err(MissingNodeInfo {
                    node_id: *node_id,
                    reason: format!("is not in cluster: {:?}", nodes.keys().cloned().collect::<Vec<_>>()),
                });
            }
        }

        let has_some = nodes.values().any(|x| x.is_some());
        if has_some {
            let first_none = nodes.iter().find(|(_node_id, v)| v.is_none());
            if let Some(first_none) = first_none {
                return Err(MissingNodeInfo {
                    node_id: *first_none.0,
                    reason: "is None".to_string(),
                });
            }
        }

        Ok(Membership { configs, nodes })
    }

    pub(crate) fn get_nodes(&self) -> &BTreeMap<C::NodeId, Option<Node>> {
        &self.nodes
    }

    pub(crate) fn get_node(&self, node_id: &C::NodeId) -> Option<&Node> {
        let x = self.nodes.get(node_id)?;
        x.as_ref()
    }

    /// Extends nodes btreemap with another.
    ///
    /// Node that present in `old` will **NOT** be replaced because changing the address of a node potentially breaks
    /// consensus guarantee.
    pub(crate) fn extend_nodes(
        old: BTreeMap<C::NodeId, Option<Node>>,
        new: &BTreeMap<C::NodeId, Option<Node>>,
    ) -> BTreeMap<C::NodeId, Option<Node>> {
        let mut res = old;

        for (k, v) in new.iter() {
            if res.contains_key(k) {
                continue;
            }
            res.insert(*k, v.clone());
        }

        res
    }

    pub fn get_configs(&self) -> &Vec<BTreeSet<C::NodeId>> {
        &self.configs
    }

    /// Check to see if the config is currently in joint consensus.
    pub fn is_in_joint_consensus(&self) -> bool {
        self.configs.len() > 1
    }

    pub(crate) fn add_learner(&self, node_id: C::NodeId, node: Option<Node>) -> Result<Self, MissingNodeInfo<C>> {
        let configs = self.configs.clone();

        let nodes = Self::extend_nodes(self.nodes.clone(), &btreemap! {node_id=>node});

        let m = Self::with_nodes(configs, nodes)?;

        Ok(m)
    }

    #[allow(dead_code)]
    pub(crate) fn learner_ids(&self) -> impl Iterator<Item = &C::NodeId> {
        self.nodes.keys().filter(|x| !self.is_member(x))
    }

    pub(crate) fn node_ids(&self) -> impl Iterator<Item = &C::NodeId> {
        self.nodes.keys()
    }

    pub(crate) fn build_member_ids(&self) -> BTreeSet<C::NodeId> {
        Self::build_all_member_ids(&self.configs)
    }

    pub(crate) fn contains(&self, node_id: &C::NodeId) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Check if the given `NodeId` exists in this membership config.
    pub(crate) fn is_member(&self, node_id: &C::NodeId) -> bool {
        for c in self.configs.iter() {
            if c.contains(node_id) {
                return true;
            }
        }
        false
    }

    #[allow(dead_code)]
    pub(crate) fn is_learner(&self, node_id: &C::NodeId) -> bool {
        self.contains(node_id) && !self.is_member(node_id)
    }

    // TODO(xp): rename this
    /// Create a new initial config containing only the given node ID.
    pub(crate) fn new_initial(node_id: C::NodeId) -> Self {
        Membership::new(vec![btreeset! {node_id}], None)
    }

    /// Return true if the given set of ids constitutes a majority.
    ///
    /// I.e. the id set includes a majority of every config.
    pub(crate) fn is_majority(&self, granted: &BTreeSet<C::NodeId>) -> bool {
        for config in self.configs.iter() {
            if !Self::is_majority_of_single_config(granted, config) {
                return false;
            }
        }

        true
    }

    /// Returns the greatest value that presents in `values` that constitutes a joint majority.
    ///
    /// E.g., for a given membership: [{1,2,3}, {4,5,6}], and a value set: {1:10, 2:20, 5:20, 6:20},
    /// `10` constitutes a majoirty in the first config {1,2,3}.
    /// `20` constitutes a majority in the second config {4,5,6}.
    /// Thus the minimal value `10` is the greatest joint majority for this membership config.
    pub(crate) fn greatest_majority_value<'v, V>(&self, values: &'v BTreeMap<C::NodeId, V>) -> Option<&'v V>
    where V: Ord {
        let mut res = vec![];
        for config in self.configs.iter() {
            let mut vs = Vec::with_capacity(config.len());

            for id in config.iter() {
                let v = values.get(id);
                if let Some(v) = v {
                    vs.push(v)
                }
            }

            let majority = quorum::majority_of(config.len());

            if vs.len() < majority {
                res.push(None);
                continue;
            }

            vs.sort_unstable();

            let majority_greatest = Some(vs[vs.len() - majority]);
            res.push(majority_greatest);
        }

        let min_greatest = res.into_iter().min();
        min_greatest.unwrap_or(None)
    }

    /// Check if the `other` membership is safe to change to.
    ///
    /// Read more about:
    /// [safe-membership-change](https://datafuselabs.github.io/openraft/dynamic-membership.html#the-safe-to-relation)
    // TODO(xp): not used.
    #[allow(dead_code)]
    pub(crate) fn is_safe_to(&self, other: &Self) -> bool {
        for d in &other.configs {
            if self.configs.contains(d) {
                return true;
            }
        }

        false
    }

    /// Returns the next safe membership to change to while the expected final membership is `goal`.
    ///
    /// E.g.(`cicj` is a joint membership of `ci` and `cj`):
    /// - `c1.next_step(c1)` returns `c1`
    /// - `c1.next_step(c2)` returns `c1c2`
    /// - `c1c2.next_step(c2)` returns `c2`
    /// - `c1c2.next_step(c1)` returns `c1`
    /// - `c1c2.next_step(c3)` returns `c2c3`
    ///
    /// With this method the membership change algo is simplified to:
    /// ```ignore
    /// while curr != goal {
    ///     let next = curr.next_step(goal);
    ///     change_membership(next);
    ///     curr = next;
    /// }
    /// ```
    pub(crate) fn next_safe<T>(&self, goal: T, turn_to_learner: bool) -> Result<Self, MissingNodeInfo<C>>
    where T: IntoOptionNodes<C::NodeId> {
        let goal = goal.into_option_nodes();

        let goal_ids = goal.keys().cloned().collect::<BTreeSet<_>>();

        let new_configs = if self.configs.contains(&goal_ids) {
            vec![goal_ids]
        } else {
            vec![self.configs.last().cloned().unwrap(), goal_ids]
        };

        let mut new_nodes = Self::extend_nodes(self.nodes.clone(), &goal);

        if !turn_to_learner {
            let old_members = Self::build_all_member_ids(&self.configs);
            let new_members = Self::build_all_member_ids(&new_configs);
            let not_in_new = old_members.difference(&new_members);

            for node_id in not_in_new {
                new_nodes.remove(node_id);
            }
        };

        let m = Membership::with_nodes(new_configs, new_nodes)?;
        Ok(m)
    }

    fn is_majority_of_single_config(granted: &BTreeSet<C::NodeId>, single_config: &BTreeSet<C::NodeId>) -> bool {
        let d = granted.intersection(single_config);
        let n_granted = d.fold(0, |a, _x| a + 1);

        let majority = quorum::majority_of(single_config.len());
        n_granted >= majority
    }

    fn build_all_member_ids(configs: &[BTreeSet<C::NodeId>]) -> BTreeSet<C::NodeId> {
        let mut members = BTreeSet::new();
        for config in configs.iter() {
            members.extend(config)
        }
        members
    }
}
