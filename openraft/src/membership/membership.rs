use core::cmp::Ord;
use core::option::Option;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreeset;
use serde::Deserialize;
use serde::Serialize;

use crate::membership::quorum;
use crate::MessageSummary;
use crate::NodeId;

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset of a majority of
/// every config.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// Learners set
    learners: BTreeSet<NodeId>,

    /// Multi configs of members.
    ///
    /// AKA a joint config in original raft paper.
    configs: Vec<BTreeSet<NodeId>>,
}

impl MessageSummary for Membership {
    fn summary(&self) -> String {
        let mut res = vec!["members:[".to_string()];
        for (i, c) in self.configs.iter().enumerate() {
            if i > 0 {
                res.push(",".to_string());
            }
            res.push(format!("{:?}", c));
        }
        res.push("]".to_string());

        res.push(",learners:[".to_string());
        for (learner_cnt, learner_id) in self.learners.iter().enumerate() {
            if learner_cnt > 0 {
                res.push(",".to_string());
            }
            res.push(format!("{:?}", learner_id));
        }
        res.push("]".to_string());
        res.join("")
    }
}

impl Membership {
    /// Create a new Membership of multiple configs(joint) and optinally a set of learners
    ///
    /// A learner that is already in configs will be filtered out.
    pub fn new(configs: Vec<BTreeSet<NodeId>>, learners: Option<BTreeSet<NodeId>>) -> Self {
        let all_members = Self::build_all_members(&configs);
        let learners = learners.unwrap_or_default();
        let learners = learners.difference(&all_members).cloned().collect::<BTreeSet<_>>();
        Membership { learners, configs }
    }

    pub fn get_configs(&self) -> &Vec<BTreeSet<NodeId>> {
        &self.configs
    }

    /// Check to see if the config is currently in joint consensus.
    pub fn is_in_joint_consensus(&self) -> bool {
        self.configs.len() > 1
    }

    #[must_use]
    pub(crate) fn add_learner(&self, id: &NodeId) -> Self {
        let mut learners = self.learners.clone();
        learners.insert(*id);
        let configs = self.configs.clone();
        Self::new(configs, Some(learners))
    }

    pub(crate) fn all_learners(&self) -> &BTreeSet<NodeId> {
        &self.learners
    }

    pub(crate) fn all_members(&self) -> BTreeSet<NodeId> {
        Self::build_all_members(&self.configs)
    }

    pub(crate) fn get_ith_config(&self, i: usize) -> Option<&BTreeSet<NodeId>> {
        self.configs.get(i)
    }

    pub(crate) fn contains(&self, target: &NodeId) -> bool {
        self.is_member(target) || self.is_learner(target)
    }

    /// Check if the given NodeId exists in this membership config.
    pub(crate) fn is_member(&self, x: &NodeId) -> bool {
        for c in self.configs.iter() {
            if c.contains(x) {
                return true;
            }
        }
        false
    }

    pub(crate) fn is_learner(&self, x: &NodeId) -> bool {
        self.learners.contains(x)
    }

    // TODO(xp): rename this
    /// Create a new initial config containing only the given node ID.
    pub(crate) fn new_initial(id: NodeId) -> Self {
        Membership::new(vec![btreeset! {id}], None)
    }

    /// Return true if the given set of ids constitutes a majority.
    ///
    /// I.e. the id set includes a majority of every config.
    pub(crate) fn is_majority(&self, granted: &BTreeSet<NodeId>) -> bool {
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
    pub(crate) fn greatest_majority_value<'v, V>(&self, values: &'v BTreeMap<NodeId, V>) -> Option<&'v V>
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
    #[must_use]
    pub(crate) fn next_safe(&self, goal: BTreeSet<NodeId>, turn_to_learner: bool) -> Self {
        let new_configs = if self.configs.contains(&goal) {
            vec![goal]
        } else {
            vec![self.configs.last().cloned().unwrap(), goal]
        };

        let mut new_learners = self.all_learners().clone();

        if turn_to_learner {
            // Add all members as learners.
            // The learners that are already a member will be filtered out in Self::new()
            new_learners.append(&mut self.all_members());
        };

        Membership::new(new_configs, Some(new_learners))
    }

    fn is_majority_of_single_config(granted: &BTreeSet<NodeId>, single_config: &BTreeSet<NodeId>) -> bool {
        let d = granted.intersection(single_config);
        let n_granted = d.fold(0, |a, _x| a + 1);

        let majority = quorum::majority_of(single_config.len());
        n_granted >= majority
    }

    fn build_all_members(configs: &[BTreeSet<NodeId>]) -> BTreeSet<NodeId> {
        let mut members = BTreeSet::new();
        for config in configs.iter() {
            members.extend(config)
        }
        members
    }
}
