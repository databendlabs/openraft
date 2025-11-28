//! Errors specific to Multi-Raft operations.

use crate::multi_raft::GroupId;

/// Errors specific to managing multiple Raft groups and routing requests between them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum MultiRaftError<G, N>
where
    G: GroupId,
    N: crate::NodeId,
{
    /// The specified Raft group was not found.
    ///
    /// This occurs when trying to access a group that hasn't been registered
    /// with the router or has been removed.
    #[error("Raft group not found: {group_id}")]
    GroupNotFound {
        /// The group ID that was not found.
        group_id: G,
    },

    /// The specified node was not found in the given group.
    ///
    /// This occurs when trying to access a specific node within a group,
    /// but that node isn't registered.
    #[error("Node {node_id} not found in group {group_id}")]
    NodeNotFound {
        /// The group ID.
        group_id: G,
        /// The node ID that was not found.
        node_id: N,
    },

    /// The node is already registered in the router.
    ///
    /// Each (group_id, node_id) pair can only be registered once.
    #[error("Node {node_id} is already registered in group {group_id}")]
    AlreadyRegistered {
        /// The group ID.
        group_id: G,
        /// The node ID that is already registered.
        node_id: N,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_group_not_found() {
        let err: MultiRaftError<String, u64> = MultiRaftError::GroupNotFound {
            group_id: "my_group".to_string(),
        };
        assert_eq!(err.to_string(), "Raft group not found: my_group");
    }

    #[test]
    fn test_error_display_node_not_found() {
        let err: MultiRaftError<String, u64> = MultiRaftError::NodeNotFound {
            group_id: "my_group".to_string(),
            node_id: 42,
        };
        assert_eq!(err.to_string(), "Node 42 not found in group my_group");
    }

    #[test]
    fn test_error_display_already_registered() {
        let err: MultiRaftError<String, u64> = MultiRaftError::AlreadyRegistered {
            group_id: "my_group".to_string(),
            node_id: 42,
        };
        assert_eq!(err.to_string(), "Node 42 is already registered in group my_group");
    }

    #[test]
    fn test_error_is_std_error() {
        fn assert_std_error<E: std::error::Error>() {}
        assert_std_error::<MultiRaftError<String, u64>>();
    }

    #[test]
    fn test_error_with_numeric_group_id() {
        let err: MultiRaftError<u64, u64> = MultiRaftError::GroupNotFound { group_id: 12345 };
        assert_eq!(err.to_string(), "Raft group not found: 12345");
    }

    #[test]
    fn test_error_equality() {
        let err1: MultiRaftError<String, u64> = MultiRaftError::GroupNotFound {
            group_id: "test".to_string(),
        };
        let err2: MultiRaftError<String, u64> = MultiRaftError::GroupNotFound {
            group_id: "test".to_string(),
        };
        let err3: MultiRaftError<String, u64> = MultiRaftError::GroupNotFound {
            group_id: "other".to_string(),
        };

        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }
}
