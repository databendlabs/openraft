use core::cmp::Ord;
use core::option::Option;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use serde::Deserialize;
use serde::Serialize;

use crate::error::NodeIdNotInNodes;
use crate::membership::quorum;
use crate::MessageSummary;
use crate::Node;
use crate::RaftTypeConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EitherNodesOrIds<C: RaftTypeConfig> {
    Nodes(BTreeMap<C::NodeId, Node>),
    NodeIds(BTreeSet<C::NodeId>),
}

impl<C: RaftTypeConfig> From<BTreeSet<C::NodeId>> for EitherNodesOrIds<C> {
    fn from(node_ids: BTreeSet<C::NodeId>) -> Self {
        Self::NodeIds(node_ids)
    }
}
impl<C: RaftTypeConfig> From<BTreeMap<C::NodeId, Node>> for EitherNodesOrIds<C> {
    fn from(nodes: BTreeMap<C::NodeId, Node>) -> Self {
        Self::Nodes(nodes)
    }
}

impl<C: RaftTypeConfig> EitherNodesOrIds<C> {
    pub fn node_ids(&self) -> BTreeSet<C::NodeId> {
        match self {
            EitherNodesOrIds::NodeIds(ids) => ids.clone(),
            EitherNodesOrIds::Nodes(nodes) => nodes.keys().cloned().collect::<BTreeSet<_>>(),
        }
    }

    pub fn nodes(&self) -> Option<BTreeMap<C::NodeId, Node>> {
        match self {
            EitherNodesOrIds::NodeIds(_ids) => None,
            EitherNodesOrIds::Nodes(nodes) => Some(nodes.clone()),
        }
    }
}

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset of a majority of
/// every config.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership<C: RaftTypeConfig> {
    /// Learners set
    learners: BTreeSet<C::NodeId>,

    /// Multi configs of members.
    ///
    /// AKA a joint config in original raft paper.
    configs: Vec<BTreeSet<C::NodeId>>,

    /// Nodes info, e.g. the connecting host and port.
    ///
    /// If it is Some, it has to contain all node id in `configs` and `learners`.
    nodes: Option<BTreeMap<C::NodeId, Node>>,
}

impl<C: RaftTypeConfig> MessageSummary for Membership<C> {
    fn summary(&self) -> String {
        let nodes = self.get_nodes();

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

                if let Some(ns) = nodes {
                    res.push(format!(":{{{}}}", ns.get(node_id).unwrap()));
                }
            }
            res.push("}".to_string());
        }
        res.push("]".to_string());

        res.push(",learners:[".to_string());
        for (learner_cnt, learner_id) in self.learners.iter().enumerate() {
            if learner_cnt > 0 {
                res.push(",".to_string());
            }

            res.push(format!("{}", learner_id));

            if let Some(ns) = nodes {
                res.push(format!(":{{{}}}", ns.get(learner_id).unwrap()));
            }
        }
        res.push("]".to_string());
        res.join("")
    }
}

impl<C: RaftTypeConfig> Membership<C> {
    /// Create a new Membership of multiple configs(joint) and optinally a set of learners
    ///
    /// A learner that is already in configs will be filtered out.
    pub fn new(configs: Vec<BTreeSet<C::NodeId>>, learners: Option<BTreeSet<C::NodeId>>) -> Self {
        let all_members = Self::build_all_members(&configs);
        let learners = learners.unwrap_or_default();
        let learners = learners.difference(&all_members).cloned().collect::<BTreeSet<_>>();

        Membership {
            learners,
            configs,
            nodes: None,
        }
    }

    /// Attatch nodes info to a Membership and returns a new instance.
    ///
    /// The provided `nodes` has to contain every node-id presents in `configs` or `learners` if it is `Some`.
    pub(crate) fn set_nodes(mut self, nodes: Option<BTreeMap<C::NodeId, Node>>) -> Result<Self, NodeIdNotInNodes<C>> {
        self.check_node_ids_in_nodes(&nodes)?;

        self.nodes = self.remove_unused_nodes(&nodes);

        Ok(self)
    }

    #[allow(dead_code)]
    pub(crate) fn get_nodes(&self) -> &Option<BTreeMap<C::NodeId, Node>> {
        &self.nodes
    }

    // TODO(xp)
    #[allow(dead_code)]
    pub(crate) fn get_node(&self, node_id: C::NodeId) -> Option<&Node> {
        if let Some(ns) = &self.nodes {
            return ns.get(&node_id);
        }

        None
    }

    pub(crate) fn remove_unused_nodes(
        &self,
        nodes: &Option<BTreeMap<C::NodeId, Node>>,
    ) -> Option<BTreeMap<C::NodeId, Node>> {
        let ns = match nodes {
            None => {
                return None;
            }
            Some(x) => x,
        };

        let mut c = btreemap! {};

        let all_members = self.all_members();
        let learners = self.all_learners();

        for node_id in all_members.iter().chain(learners) {
            c.insert(*node_id, ns[node_id].clone());
        }

        Some(c)
    }

    /// Extends nodes with another.
    ///
    /// Node that present in `old` will not be replaced because changing the address of a node potentially breaks
    /// consensus guarantee.
    pub fn extend_nodes(
        old: Option<BTreeMap<C::NodeId, Node>>,
        new: &Option<BTreeMap<C::NodeId, Node>>,
    ) -> Option<BTreeMap<C::NodeId, Node>> {
        let new_nodes = match new {
            None => return old,
            Some(x) => x,
        };

        let mut res = old.unwrap_or_default();

        for (k, v) in new_nodes.iter() {
            if res.contains_key(k) {
                continue;
            }
            res.insert(*k, v.clone());
        }

        Some(res)
    }

    /// Check if all member node-id and learner node id are included in `self.nodes`, if `self.nodes` is `Some`.
    pub(crate) fn check_node_ids_in_nodes(
        &self,
        nodes: &Option<BTreeMap<C::NodeId, Node>>,
    ) -> Result<(), NodeIdNotInNodes<C>> {
        let ns = match nodes {
            None => return Ok(()),
            Some(x) => x,
        };

        let all_members = self.all_members();

        for node_id in all_members.iter().chain(self.learners.iter()) {
            if !ns.contains_key(node_id) {
                let e = NodeIdNotInNodes {
                    node_id: *node_id,
                    node_ids: ns.keys().cloned().collect::<BTreeSet<C::NodeId>>(),
                };
                return Err(e);
            }
        }

        Ok(())
    }

    pub fn get_configs(&self) -> &Vec<BTreeSet<C::NodeId>> {
        &self.configs
    }

    /// Check to see if the config is currently in joint consensus.
    pub fn is_in_joint_consensus(&self) -> bool {
        self.configs.len() > 1
    }

    pub(crate) fn add_learner(&self, node_id: C::NodeId, node: Option<Node>) -> Result<Self, NodeIdNotInNodes<C>> {
        let extension = node.map(|n| btreemap! {node_id=>n});

        let configs = self.configs.clone();
        let mut learners = self.learners.clone();

        learners.insert(node_id);

        let nodes = Self::extend_nodes(self.nodes.clone(), &extension);

        let m = Self::new(configs, Some(learners));
        let m = m.set_nodes(nodes)?;

        Ok(m)
    }

    pub(crate) fn all_learners(&self) -> &BTreeSet<C::NodeId> {
        &self.learners
    }

    pub(crate) fn all_members(&self) -> BTreeSet<C::NodeId> {
        Self::build_all_members(&self.configs)
    }

    pub(crate) fn get_ith_config(&self, i: usize) -> Option<&BTreeSet<C::NodeId>> {
        self.configs.get(i)
    }

    pub(crate) fn contains(&self, target: &C::NodeId) -> bool {
        self.is_member(target) || self.is_learner(target)
    }

    /// Check if the given `NodeId` exists in this membership config.
    pub(crate) fn is_member(&self, x: &C::NodeId) -> bool {
        for c in self.configs.iter() {
            if c.contains(x) {
                return true;
            }
        }
        false
    }

    pub(crate) fn is_learner(&self, x: &C::NodeId) -> bool {
        self.learners.contains(x)
    }

    // TODO(xp): rename this
    /// Create a new initial config containing only the given node ID.
    pub(crate) fn new_initial(id: C::NodeId) -> Self {
        Membership::new(vec![btreeset! {id}], None)
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
    /// - `c1.next_step(c2)` returns `c1*c2`
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
    pub(crate) fn next_safe<T>(&self, goal: T, turn_to_learner: bool) -> Result<Self, NodeIdNotInNodes<C>>
    where T: Into<EitherNodesOrIds<C>> {
        let goal: EitherNodesOrIds<C> = goal.into();

        let goal_ids = goal.node_ids();

        let new_configs = if self.configs.contains(&goal_ids) {
            vec![goal_ids]
        } else {
            vec![self.configs.last().cloned().unwrap(), goal_ids]
        };

        let mut new_learners = self.all_learners().clone();

        if turn_to_learner {
            // Add all members as learners.
            // The learners that are already a member will be filtered out in Self::new()
            new_learners.append(&mut self.all_members());
        };

        let new_nodes = Self::extend_nodes(self.nodes.clone(), &goal.nodes());

        let m = Membership::new(new_configs, Some(new_learners));
        let res = m.set_nodes(new_nodes)?;
        Ok(res)
    }

    fn is_majority_of_single_config(granted: &BTreeSet<C::NodeId>, single_config: &BTreeSet<C::NodeId>) -> bool {
        let d = granted.intersection(single_config);
        let n_granted = d.fold(0, |a, _x| a + 1);

        let majority = quorum::majority_of(single_config.len());
        n_granted >= majority
    }

    fn build_all_members(configs: &[BTreeSet<C::NodeId>]) -> BTreeSet<C::NodeId> {
        let mut members = BTreeSet::new();
        for config in configs.iter() {
            members.extend(config)
        }
        members
    }
}
