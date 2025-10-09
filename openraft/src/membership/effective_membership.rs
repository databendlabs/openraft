use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;

use crate::Membership;
use crate::RaftTypeConfig;
use crate::StoredMembership;
use crate::display_ext::DisplayOptionExt;
use crate::log_id::raft_log_id::RaftLogId;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::type_config::alias::LogIdOf;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Clone, Default, Eq)]
pub struct EffectiveMembership<C>
where C: RaftTypeConfig
{
    stored_membership: Arc<StoredMembership<C>>,

    /// The quorum set built from `membership`.
    quorum_set: Joint<C::NodeId, Vec<C::NodeId>, Vec<Vec<C::NodeId>>>,

    /// Cache of the union of all members
    voter_ids: BTreeSet<C::NodeId>,
}

impl<C> Debug for EffectiveMembership<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectiveMembership")
            .field("log_id", self.log_id())
            .field("membership", self.membership())
            .field("voter_ids", &self.voter_ids)
            .finish()
    }
}

impl<C> PartialEq for EffectiveMembership<C>
where C: RaftTypeConfig
{
    fn eq(&self, other: &Self) -> bool {
        self.stored_membership == other.stored_membership && self.voter_ids == other.voter_ids
    }
}

impl<C, LID> From<(&LID, Membership<C>)> for EffectiveMembership<C>
where
    C: RaftTypeConfig,
    LID: RaftLogId<C>,
{
    fn from(v: (&LID, Membership<C>)) -> Self {
        EffectiveMembership::new(Some(v.0.to_log_id()), v.1)
    }
}

impl<C> EffectiveMembership<C>
where C: RaftTypeConfig
{
    pub(crate) fn new_arc(log_id: Option<LogIdOf<C>>, membership: Membership<C>) -> Arc<Self> {
        Arc::new(Self::new(log_id, membership))
    }

    /// Create a new EffectiveMembership from a log ID and membership configuration.
    pub fn new(log_id: Option<LogIdOf<C>>, membership: Membership<C>) -> Self {
        let voter_ids = membership.voter_ids().collect();

        let configs = membership.get_joint_config();
        let mut joint = vec![];
        for c in configs {
            joint.push(c.iter().cloned().collect::<Vec<_>>());
        }

        let quorum_set = Joint::from(joint);

        Self {
            stored_membership: Arc::new(StoredMembership::new(log_id, membership)),
            quorum_set,
            voter_ids,
        }
    }

    pub(crate) fn new_from_stored_membership(stored: StoredMembership<C>) -> Self {
        Self::new(stored.log_id().clone(), stored.membership().clone())
    }

    pub(crate) fn stored_membership(&self) -> &Arc<StoredMembership<C>> {
        &self.stored_membership
    }

    /// Get the log ID at which this membership was stored.
    pub fn log_id(&self) -> &Option<LogIdOf<C>> {
        self.stored_membership.log_id()
    }

    /// Get the membership configuration.
    pub fn membership(&self) -> &Membership<C> {
        self.stored_membership.membership()
    }
}

/// Membership API
impl<C> EffectiveMembership<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
    pub(crate) fn is_voter(&self, nid: &C::NodeId) -> bool {
        self.membership().is_voter(nid)
    }

    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub fn voter_ids(&self) -> impl Iterator<Item = C::NodeId> + '_ {
        self.voter_ids.iter().cloned()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    pub(crate) fn learner_ids(&self) -> impl Iterator<Item = C::NodeId> + '_ {
        self.membership().learner_ids()
    }

    /// Get the node (either voter or learner) by node id.
    pub fn get_node(&self, node_id: &C::NodeId) -> Option<&C::Node> {
        self.membership().get_node(node_id)
    }

    /// Returns an Iterator of all nodes (voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&C::NodeId, &C::Node)> {
        self.membership().nodes()
    }

    /// Returns reference to the joint config.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a vec of node-id.
    pub fn get_joint_config(&self) -> &Vec<Vec<C::NodeId>> {
        self.quorum_set.children()
    }
}

impl<C> fmt::Display for EffectiveMembership<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EffectiveMembership{{log_id: {}, membership: {}}}",
            self.log_id().display(),
            self.membership()
        )
    }
}

/// Implement node-id joint quorum set.
impl<C> QuorumSet<C::NodeId> for EffectiveMembership<C>
where C: RaftTypeConfig
{
    type Iter = std::collections::btree_set::IntoIter<C::NodeId>;

    fn is_quorum<'a, I: Iterator<Item = &'a C::NodeId> + Clone>(&self, ids: I) -> bool {
        self.quorum_set.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.quorum_set.ids()
    }
}
