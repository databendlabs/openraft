//! Node identification and metadata.
//!
//! This module defines traits and types for representing nodes in a Raft cluster.
//!
//! ## Key Traits
//!
//! - [`NodeId`] - Unique identifier for a node (typically `u64`)
//! - [`Node`] - Node information including network address and metadata
//!
//! ## Implementations
//!
//! - [`BasicNode`] - Node with network address string
//! - [`EmptyNode`] - Minimal node with no metadata
//!
//! ## Overview
//!
//! Each Raft node is identified by a [`NodeId`] and has associated [`Node`] information:
//! - **NodeId**: Unique identifier used throughout the protocol
//! - **Node**: Additional metadata (e.g., network address) for connecting to the node
//!
//! Applications can use built-in types or define custom [`Node`] implementations to store
//! additional metadata like datacenter location, priority, or capabilities.

use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::hash::Hash;

use crate::base::OptionalFeatures;

/// A Raft node's ID.
///
/// A `NodeId` uniquely identifies a node in the Raft cluster.
pub trait NodeId
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

impl<T> NodeId for T where T: Sized
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

/// A Raft `Node`, this trait holds all relevant node information.
///
/// For the most generic case [`BasicNode`] provides an example implementation including the node's
/// network address, but the used [`Node`] implementation can be customized to include additional
/// information.
pub trait Node
where Self: Sized + OptionalFeatures + Eq + PartialEq + Debug + Clone + 'static
{
}

impl<T> Node for T where T: Sized + OptionalFeatures + Eq + PartialEq + Debug + Clone + 'static {}

/// EmptyNode is an implementation of the [`Node`] trait that contains nothing.
///
/// Such a node stores nothing but is just a placeholder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct EmptyNode {}

impl EmptyNode {
    /// Creates an [`EmptyNode`].
    pub fn new() -> Self {
        Self {}
    }
}

impl Display for EmptyNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{}}")
    }
}

/// An implementation of the [`Node`] trait that contains minimal node information.
///
/// The most common usage is to store the connecting address of a node.
/// So that an application does not need an additional store to support its
/// [`RaftNetwork`](crate::RaftNetwork) implementation.
///
/// An application is also free not to use this storage and implements its own node-id to address
/// mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct BasicNode {
    /// A user-defined string that represents the endpoint of the target node.
    ///
    /// It is used by [`RaftNetwork`](crate::RaftNetwork) for connecting to the target node.
    pub addr: String,
}

impl BasicNode {
    /// Creates as [`BasicNode`].
    pub fn new(addr: impl ToString) -> Self {
        Self { addr: addr.to_string() }
    }
}

impl Display for BasicNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use crate::NodeId;

    #[test]
    fn node_id_default_impl() {
        /// Automatically implemented trait [`NodeId`] for this struct.
        #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Ord, PartialOrd)]
        struct AutoNodeId;

        impl fmt::Display for AutoNodeId {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "FooNodeId")
            }
        }

        /// Assert a value implements the trait [`NodeId`].
        fn assert_node_id<NID: NodeId>(_nid: &NID) {}

        assert_node_id(&AutoNodeId);
    }
}
