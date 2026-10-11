use openraft_macros::since;

use crate::Node;
use crate::membership::NodeRole;

/// A known node's metadata, capabilities, and quorum membership.
///
/// [`RaftMembership::get_node()`](crate::membership::RaftMembership::get_node) derives the voter
/// flags from the read and write quorum sets. Every write voter must have
/// [`NodeRole::ReceiveLogs`].
#[since(version = "0.10.0")]
#[derive(Debug, PartialEq, Eq)]
pub struct NodeRef<'s, N>
where N: Node
{
    /// The node's metadata.
    pub node: &'s N,

    /// The node's campaign and log replication capabilities.
    pub role: NodeRole,

    /// Whether the node's ID appears in the read quorum set.
    pub is_read_voter: bool,

    /// Whether the node's ID appears in the write quorum set.
    pub is_write_voter: bool,
}

impl<N> NodeRef<'_, N>
where N: Node
{
    /// Returns whether the node receives logs and has no campaign or voter role.
    ///
    /// ```
    /// use openraft::EmptyNode;
    /// use openraft::membership::NodeRef;
    /// use openraft::membership::NodeRole;
    ///
    /// let metadata = EmptyNode::new();
    /// let node = NodeRef {
    ///     node: &metadata,
    ///     role: NodeRole::ReceiveLogs,
    ///     is_read_voter: false,
    ///     is_write_voter: false,
    /// };
    /// let is_learner = node.is_learner();
    /// assert!(is_learner);
    /// ```
    #[since(version = "0.10.0")]
    pub fn is_learner(&self) -> bool {
        let receives_logs = self.role.contains(NodeRole::ReceiveLogs);
        if !receives_logs {
            return false;
        }

        let is_campaigner = self.role.contains(NodeRole::Campaigner);
        let is_voter = self.is_read_voter || self.is_write_voter;
        let participates = is_campaigner || is_voter;
        !participates
    }
}

#[cfg(test)]
mod tests {
    use crate::EmptyNode;
    use crate::membership::NodeRef;
    use crate::membership::NodeRole;

    #[test]
    fn learner_requires_only_log_replication() {
        let metadata = EmptyNode::new();
        let cases = [
            (NodeRole::Empty, false),
            (NodeRole::Campaigner, false),
            (NodeRole::ReceiveLogs, true),
            (NodeRole::Campaigner | NodeRole::ReceiveLogs, false),
        ];
        for (role, expected) in cases {
            let node = NodeRef {
                node: &metadata,
                role,
                is_read_voter: false,
                is_write_voter: false,
            };
            let actual = node.is_learner();
            assert_eq!(actual, expected, "{role:?}");
        }
    }

    #[test]
    fn voters_are_not_learners() {
        let metadata = EmptyNode::new();
        let roles = [
            NodeRole::Empty,
            NodeRole::Campaigner,
            NodeRole::ReceiveLogs,
            NodeRole::Campaigner | NodeRole::ReceiveLogs,
        ];
        for role in roles {
            for (is_read_voter, is_write_voter) in [(true, false), (false, true), (true, true)] {
                let node = NodeRef {
                    node: &metadata,
                    role,
                    is_read_voter,
                    is_write_voter,
                };
                let actual = node.is_learner();
                assert!(!actual, "{role:?}, read={is_read_voter}, write={is_write_voter}");
            }
        }
    }
}
