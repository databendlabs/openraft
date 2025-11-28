//! Multi-Raft type configuration.
//!
//! This module defines [`MultiRaftTypeConfig`], which extends [`RaftTypeConfig`]
//! with support for multiple Raft groups.

use super::GroupId;
use crate::RaftTypeConfig;

/// Type configuration for Multi-Raft deployments.
///
/// This trait extends [`RaftTypeConfig`] with an additional [`GroupId`] type
/// for identifying different Raft groups within the same process.
///
/// ## Usage
///
/// After defining your base `RaftTypeConfig`, implement this trait to add
/// Multi-Raft support:
///
/// ```ignore
/// use openraft::multi_raft::{GroupId, MultiRaftTypeConfig};
///
/// // First, define base type config
/// openraft::declare_raft_types!(
///     pub MyConfig:
///         D = ClientRequest,
///         R = ClientResponse,
///         NodeId = u64,
/// );
///
/// // Then add Multi-Raft support
/// impl MultiRaftTypeConfig for MyConfig {
///     type GroupId = String;
/// }
/// ```
///
/// ## GroupId Selection
///
/// Choose a `GroupId` type based on your use case:
///
/// - **`String`**: Readable names like "region_1", "table_users"
/// - **`u64`**: Efficient numeric IDs, good for auto-generated groups
/// - **Custom struct**: Rich identifiers with multiple fields (datacenter, region, etc.)
///
/// ## Example with Custom GroupId
///
/// ```ignore
/// use std::fmt;
///
/// #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
/// struct ShardId {
///     table_id: u64,
///     shard_num: u32,
/// }
///
/// impl fmt::Display for ShardId {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "table_{}_shard_{}", self.table_id, self.shard_num)
///     }
/// }
///
/// impl MultiRaftTypeConfig for MyConfig {
///     type GroupId = ShardId;
/// }
/// ```
///
/// [`RaftTypeConfig`]: crate::RaftTypeConfig
pub trait MultiRaftTypeConfig: RaftTypeConfig {
    /// The type used to identify Raft groups.
    ///
    /// Each Raft group on a node must have a unique `GroupId`.
    /// This is used to:
    /// - Route incoming RPC requests to the correct Raft instance
    /// - Identify groups in metrics and logging
    /// - Manage group lifecycle (create, shutdown)
    type GroupId: GroupId;
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::io::Cursor;

    use super::GroupId;
    use super::MultiRaftTypeConfig;
    use crate::OptionalSend;
    use crate::RaftTypeConfig;

    /// Test type config for Multi-Raft testing
    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    struct TestConfig;

    impl RaftTypeConfig for TestConfig {
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

    // Test with String GroupId
    impl MultiRaftTypeConfig for TestConfig {
        type GroupId = String;
    }

    #[test]
    fn test_multi_raft_type_config_with_string() {
        fn assert_multi_raft_config<C: MultiRaftTypeConfig>() {}
        assert_multi_raft_config::<TestConfig>();
    }

    /// Another test config with u64 GroupId
    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    struct TestConfigNumeric;

    impl RaftTypeConfig for TestConfigNumeric {
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

    impl MultiRaftTypeConfig for TestConfigNumeric {
        type GroupId = u64;
    }

    #[test]
    fn test_multi_raft_type_config_with_u64() {
        fn assert_multi_raft_config<C: MultiRaftTypeConfig>() {}
        assert_multi_raft_config::<TestConfigNumeric>();
    }

    /// Test config with custom GroupId
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    struct RegionGroupId {
        region: u32,
        partition: u16,
    }

    impl fmt::Display for RegionGroupId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "region{}_part{}", self.region, self.partition)
        }
    }

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    struct TestConfigCustom;

    impl RaftTypeConfig for TestConfigCustom {
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

    impl MultiRaftTypeConfig for TestConfigCustom {
        type GroupId = RegionGroupId;
    }

    #[test]
    fn test_multi_raft_type_config_with_custom() {
        fn assert_multi_raft_config<C: MultiRaftTypeConfig>() {}
        assert_multi_raft_config::<TestConfigCustom>();

        fn assert_group_id<G: GroupId>(_g: &G) {}
        let gid = RegionGroupId {
            region: 1,
            partition: 5,
        };
        assert_group_id(&gid);
        assert_eq!(gid.to_string(), "region1_part5");
    }
}
