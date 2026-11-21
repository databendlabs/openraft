//! A quorum is a set of nodes whose positive responses, such as vote grants or append-entries
//! acknowledgments, are enough for a request to succeed. Nodes that were only contacted do not
//! count.
//!
//! A quorum set describes which sets of nodes are quorums. The most common one is **majority**:
//! every set that holds more than half of the voters. For the voters `{a,b,c}`, the minimal quorums
//! are `{a,b}`, `{b,c}` and `{a,c}`, and `{a,b,c}` is a quorum as well; a single node such as `{a}`
//! is not.

pub(crate) use quorum_set::BothQuorums;
pub(crate) use quorum_set::QuorumBridge;
pub(crate) use quorum_set::QuorumIntersection;
pub use quorum_set::QuorumSet;
pub(crate) use quorum_set::ReadOrBoth;
