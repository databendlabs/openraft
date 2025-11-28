//! Raft group identification.
//!
//! This module defines the [`GroupId`] trait for uniquely identifying Raft groups
//! in a Multi-Raft deployment.

use std::fmt::Debug;
use std::fmt::Display;
use std::hash::Hash;

use crate::base::OptionalFeatures;

/// A Raft group's unique identifier.
///
/// In a Multi-Raft setup, multiple Raft groups can run on the same physical node.
/// Each group is uniquely identified by a `GroupId`, which is used for:
/// - Routing RPC requests to the correct Raft instance
/// - Isolating storage between different groups
/// - Managing group membership independently
///
/// ## Common Implementations
///
/// The trait is automatically implemented for types that satisfy the required bounds:
/// - `String` - Human-readable group names (e.g., "region_1", "table_users")
/// - `u64` - Numeric identifiers for efficient storage and comparison
/// - Custom types - Application-specific identifiers
///
/// ## Example
///
/// ```
/// use std::fmt;
///
/// // Using String as GroupId
/// fn example_string_group_id() {
///     let group: String = "my_raft_group".to_string();
///     println!("Group: {}", group);
/// }
///
/// // Using u64 as GroupId
/// fn example_u64_group_id() {
///     let group: u64 = 12345;
///     println!("Group: {}", group);
/// }
///
/// // Custom GroupId type
/// #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
/// #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// struct RegionId {
///     datacenter: u16,
///     region: u32,
/// }
///
/// impl fmt::Display for RegionId {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "dc{}-region{}", self.datacenter, self.region)
///     }
/// }
/// // RegionId automatically implements GroupId
/// ```
pub trait GroupId
where Self: Sized
        + OptionalFeatures
        + Eq
        + PartialEq
        + Ord
        + PartialOrd
        + Debug
        + Display
        + Hash
        + Clone
        + Default
        + 'static
{
}

// Blanket implementation for all types satisfying the bounds
impl<T> GroupId for T where T: Sized
        + OptionalFeatures
        + Eq
        + PartialEq
        + Ord
        + PartialOrd
        + Debug
        + Display
        + Hash
        + Clone
        + Default
        + 'static
{
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::GroupId;

    /// Verifies that common types automatically implement GroupId
    #[test]
    fn test_builtin_group_id_impls() {
        fn assert_group_id<G: GroupId>(_g: &G) {}

        // String implements GroupId
        assert_group_id(&String::from("test_group"));

        // u64 implements GroupId
        assert_group_id(&42u64);

        // i64 implements GroupId
        assert_group_id(&42i64);

        // u32 implements GroupId
        assert_group_id(&42u32);
    }

    /// Custom GroupId type for testing
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    struct CustomGroupId {
        namespace: String,
        id: u64,
    }

    impl fmt::Display for CustomGroupId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}:{}", self.namespace, self.id)
        }
    }

    #[test]
    fn test_custom_group_id() {
        fn assert_group_id<G: GroupId>(_g: &G) {}

        let custom = CustomGroupId {
            namespace: "test".to_string(),
            id: 123,
        };

        assert_group_id(&custom);
        assert_eq!(custom.to_string(), "test:123");
    }
}
