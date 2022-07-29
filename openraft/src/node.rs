use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::hash::Hash;

/// Essential trait bound for node-id, except serde.
#[doc(hidden)]
pub trait NodeIdEssential:
    Sized + Send + Sync + Eq + PartialEq + Ord + PartialOrd + Debug + Display + Hash + Copy + Clone + Default + 'static
{
}

impl<T> NodeIdEssential for T where T: Sized
        + Send
        + Sync
        + Eq
        + PartialEq
        + Ord
        + PartialOrd
        + Debug
        + Display
        + Hash
        + Copy
        + Clone
        + Default
        + 'static
{
}

/// A Raft node's ID.
///
/// A `NodeId` uniquely identifies a node in the Raft cluster.
#[cfg(feature = "serde")]
pub trait NodeId: NodeIdEssential + serde::Serialize + for<'a> serde::Deserialize<'a> {}

#[cfg(feature = "serde")]
impl<T> NodeId for T where T: NodeIdEssential + serde::Serialize + for<'a> serde::Deserialize<'a> {}

#[cfg(not(feature = "serde"))]
pub trait NodeId: NodeIdEssential {}

#[cfg(not(feature = "serde"))]
impl<T> NodeId for T where T: NodeIdEssential {}

/// Essential trait bound for application level node-data, except serde.
pub trait NodeDataEssential: Sized + Send + Sync + Eq + PartialEq + Debug + Clone + Default + 'static {}
impl<T> NodeDataEssential for T where T: Sized + Send + Sync + Eq + PartialEq + Debug + Clone + Default + 'static {}

/// A Raft node's application level data.
///
/// `NodeData` holds applicaiton level information relevant to a raft node
#[cfg(feature = "serde")]
pub trait NodeData: NodeDataEssential + serde::Serialize + for<'a> serde::Deserialize<'a> {}

#[cfg(feature = "serde")]
impl<T> NodeData for T where T: NodeDataEssential + serde::Serialize + for<'a> serde::Deserialize<'a> {}

#[cfg(not(feature = "serde"))]
pub trait NodeData: NodeDataEssential {}

#[cfg(not(feature = "serde"))]
impl<T> NodeData for T where T: NodeDataEssential {}

#[cfg(feature = "serde")]
pub trait NodeType: NodeDataEssential + serde::Serialize + for<'a> serde::Deserialize<'a> + 'static {
    type NodeId: NodeId;
    /// Raft application level node data
    type NodeData: NodeData;
}

#[cfg(not(feature = "serde"))]
pub trait NodeType: NodeDataEssential + 'static {
    type NodeId: NodeId;
    /// Raft application level node data
    type NodeData: NodeData;
}
/// Additional node information.
///
/// The most common usage is to store the connecting address of a node.
/// So that an application does not need an additional store to support its RaftNetwork implementation.
///
/// An application is also free not to use this storage and implements its own node-id to address mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Node<NT>
where NT: NodeType
{
    pub addr: String,
    /// Other User defined data.
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub data: NT::NodeData,
}

impl<NT> Node<NT>
where NT: NodeType
{
    pub fn new(addr: impl ToString) -> Self {
        Self {
            addr: addr.to_string(),
            ..Default::default()
        }
    }
}

impl<NT> Display for Node<NT>
where NT: NodeType
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; {:?}", self.addr, self.data)
    }
}
