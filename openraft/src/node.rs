use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::hash::Hash;

/// A Raft node's ID.
///
/// A `NodeId` uniquely identifies a node in the Raft cluster.

#[cfg(feature = "serde_impl")]
pub trait NodeId:
    Sized
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
    + serde::Serialize
    + for<'a> serde::Deserialize<'a>
    + 'static
{
}

#[cfg(feature = "serde_impl")]
impl<T> NodeId for T where T: Sized
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
        + serde::Serialize
        + for<'a> serde::Deserialize<'a>
        + 'static
{
}

#[cfg(not(feature = "serde_impl"))]
pub trait NodeId:
    Sized + Send + Sync + Eq + PartialEq + Ord + PartialOrd + Debug + Display + Hash + Copy + Clone + Default + 'static
{
}

#[cfg(not(feature = "serde_impl"))]
impl<T> NodeId for T where T: Sized
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

/// Additional node information.
///
/// The most usage is to store the connecting address of a node.
/// So that an application does not need a 3rd party store to support its RaftNetwork implememntation.
///
/// An application is also free not to use this storage and implements its own node-id to address mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde_impl", derive(serde::Serialize, serde::Deserialize))]
pub struct Node {
    pub addr: String,
    /// Other User defined data.
    pub data: BTreeMap<String, String>,
}

impl Node {
    pub fn new(addr: impl ToString) -> Self {
        Self {
            addr: addr.to_string(),
            ..Default::default()
        }
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; ", self.addr)?;
        for (i, (k, v)) in self.data.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{}:{}", k, v)?;
        }
        Ok(())
    }
}
