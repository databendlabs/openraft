use std::collections::BTreeMap;
use std::fmt::Display;
use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;

// If GUID-based `NodeId` is activated, include the definition from submodule.
//
// NOTE: If you are using IDE with rust-analyzer with all-features turned on by default,
// turn it off for the workspace to prevent showing errors in test files, which are not
// adjusted to use `NodeId`, but rather use `u64` directly.
#[cfg(feature = "guid_nodeid")]
pub mod guid_nodeid;
#[cfg(feature = "guid_nodeid")]
pub use guid_nodeid::NodeId;
#[cfg(not(feature = "guid_nodeid"))]
/// A Raft node's ID.
pub type NodeId = u64;

/// Additional node information.
///
/// The most usage is to store the connecting address of a node.
/// So that an application does not need a 3rd party store to support its RaftNetwork implememntation.
///
/// An application is also free not to use this storage and implements its own node-id to address mapping.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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
