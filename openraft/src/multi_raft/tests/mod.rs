use std::fmt;
use std::io::Cursor;

use super::GroupId;
use super::MultiRaftError;
use super::MultiRaftTypeConfig;
use super::RaftRouter;
use crate::OptionalSend;
use crate::RaftTypeConfig;

/// A comprehensive test type config
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct TestTypeConfig;

impl RaftTypeConfig for TestTypeConfig {
    type D = String;
    type R = String;
    type NodeId = u64;
    type Node = crate::EmptyNode;
    type Term = u64;
    type LeaderId = crate::impls::leader_id_adv::LeaderId<Self>;
    type Vote = crate::impls::Vote<Self>;
    type Entry = crate::impls::Entry<Self>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = crate::impls::TokioRuntime;
    type Responder<T>
        = crate::impls::OneshotResponder<Self, T>
    where T: OptionalSend + 'static;
}

impl MultiRaftTypeConfig for TestTypeConfig {
    type GroupId = String;
}

/// Custom group ID for testing complex scenarios
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct TableShardId {
    table_id: u64,
    shard_id: u32,
}

impl fmt::Display for TableShardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "table_{}_shard_{}", self.table_id, self.shard_id)
    }
}

// ============================================================================
// GroupId Tests
// ============================================================================

#[test]
fn test_string_as_group_id() {
    fn use_group_id<G: GroupId>(g: G) -> String {
        format!("Group: {}", g)
    }

    let result = use_group_id("my_group".to_string());
    assert_eq!(result, "Group: my_group");
}

#[test]
fn test_u64_as_group_id() {
    fn use_group_id<G: GroupId>(g: G) -> String {
        format!("Group: {}", g)
    }

    let result = use_group_id(12345u64);
    assert_eq!(result, "Group: 12345");
}

#[test]
fn test_custom_group_id() {
    fn use_group_id<G: GroupId>(g: G) -> String {
        format!("Group: {}", g)
    }

    let gid = TableShardId {
        table_id: 100,
        shard_id: 5,
    };
    let result = use_group_id(gid);
    assert_eq!(result, "Group: table_100_shard_5");
}

#[test]
fn test_group_id_ordering() {
    let g1 = TableShardId {
        table_id: 1,
        shard_id: 1,
    };
    let g2 = TableShardId {
        table_id: 1,
        shard_id: 2,
    };
    let g3 = TableShardId {
        table_id: 2,
        shard_id: 1,
    };

    assert!(g1 < g2);
    assert!(g2 < g3);
    assert!(g1 < g3);
}

#[test]
fn test_group_id_hash() {
    use std::collections::HashSet;

    let mut set: HashSet<TableShardId> = HashSet::new();

    set.insert(TableShardId {
        table_id: 1,
        shard_id: 1,
    });
    set.insert(TableShardId {
        table_id: 1,
        shard_id: 2,
    });
    set.insert(TableShardId {
        table_id: 1,
        shard_id: 1,
    }); // duplicate

    assert_eq!(set.len(), 2);
}

// ============================================================================
// MultiRaftTypeConfig Tests
// ============================================================================

#[test]
fn test_multi_raft_type_config_associated_types() {
    fn check_config<C: MultiRaftTypeConfig>()
    where C::GroupId: GroupId {
        // Just verify the associated types work correctly
    }

    check_config::<TestTypeConfig>();
}

// ============================================================================
// Error Tests
// ============================================================================

#[test]
fn test_error_debug_impl() {
    let err: MultiRaftError<String, u64> = MultiRaftError::GroupNotFound {
        group_id: "test".to_string(),
    };

    let debug_str = format!("{:?}", err);
    assert!(debug_str.contains("GroupNotFound"));
    assert!(debug_str.contains("test"));
}

#[test]
fn test_error_clone() {
    let err1: MultiRaftError<String, u64> = MultiRaftError::NodeNotFound {
        group_id: "group1".to_string(),
        node_id: 42,
    };

    let err2 = err1.clone();
    assert_eq!(err1, err2);
}

// ============================================================================
// RaftRouter Tests (type-level / no Raft instance needed)
// ============================================================================

#[test]
fn test_router_new_is_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    assert!(router.is_empty());
    assert_eq!(router.len(), 0);
    assert_eq!(router.group_count(), 0);
}

#[test]
fn test_router_default() {
    let router = RaftRouter::<TestTypeConfig>::default();
    assert!(router.is_empty());
}

#[test]
fn test_router_debug() {
    let router = RaftRouter::<TestTypeConfig>::new();
    let debug_str = format!("{:?}", router);
    assert!(debug_str.contains("RaftRouter"));
    assert!(debug_str.contains("node_count"));
    assert!(debug_str.contains("group_count"));
}

#[test]
fn test_router_contains_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    assert!(!router.contains(&"group1".to_string(), &1));
    assert!(!router.contains_group(&"group1".to_string()));
}

#[test]
fn test_router_get_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    assert!(router.get(&"group1".to_string(), &1).is_none());
}

#[test]
fn test_router_get_group_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    let nodes = router.get_group(&"group1".to_string());
    assert!(nodes.is_empty());
}

#[test]
fn test_router_all_nodes_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    assert!(router.all_nodes().is_empty());
}

#[test]
fn test_router_all_groups_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    assert!(router.all_groups().is_empty());
}

#[test]
fn test_router_unregister_nonexistent() {
    let router = RaftRouter::<TestTypeConfig>::new();
    let result = router.unregister(&"group1".to_string(), &1);
    assert!(result.is_none());
}

#[test]
fn test_router_remove_group_nonexistent() {
    let router = RaftRouter::<TestTypeConfig>::new();
    let removed = router.remove_group(&"group1".to_string());
    assert!(removed.is_empty());
}

#[test]
fn test_router_clear_empty() {
    let router = RaftRouter::<TestTypeConfig>::new();
    router.clear();
    assert!(router.is_empty());
}

/// Test that RaftRouter is Send + Sync in multi-threaded mode.
///
/// In singlethreaded mode, RaftRouter is not necessarily Send + Sync,
/// which is correct for single-threaded runtimes like compio.
#[test]
#[cfg(not(feature = "singlethreaded"))]
fn test_router_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RaftRouter<TestTypeConfig>>();
}

#[test]
fn test_router_can_be_arc() {
    use std::sync::Arc;
    let router = Arc::new(RaftRouter::<TestTypeConfig>::new());
    assert!(router.is_empty());
}
