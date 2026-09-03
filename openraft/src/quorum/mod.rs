//! A quorum is a set of nodes a vote request or append-entries request has to contact to.
//! The most common quorum is **majority**.
//! A quorum set is a collection of quorums, e.g., the quorum set of the majority of `{a,b,c}` is
//! `{a,b}, {b,c}, {a,c}`.

pub(crate) use quorum_set::QuorumBridge;
pub(crate) use quorum_set::QuorumIntersection;
pub use quorum_set::QuorumSet;
