//! A quorum is a set of nodes a vote request or append-entries request has to contact to.
//! The most common quorum is **majority**.
//! A quorum set is a collection of quorums, e.g., the quorum set of the majority of `{a,b,c}` is
//! `{a,b}, {b,c}, {a,c}`.

mod joint;
mod threshold_quorum_set;

#[cfg(test)]
mod joint_test;
#[cfg(test)]
mod threshold_quorum_set_test;

pub use joint::Joint;
pub(crate) use quorum_set::BothQuorums;
pub(crate) use quorum_set::QuorumBridge;
pub(crate) use quorum_set::QuorumIntersection;
pub use quorum_set::QuorumSet;
pub(crate) use quorum_set::ReadOrBoth;
pub use threshold_quorum_set::ThresholdQuorumSet;
