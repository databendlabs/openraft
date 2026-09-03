//! Progress tracks replication state, i.e., it can be considered a map of node id to already
//! replicated log id.
//!
//! The "progress" internally is a vector of scalar values.
//! The scalar value is monotonically incremental. Decreasing it is not allowed.
//! Optimization on calculating the quorum-accepted log id is done on this assumption.

pub(crate) mod entry;
pub(crate) mod inflight;
pub(crate) mod inflight_id;
pub(crate) mod stream_id;

pub(crate) use inflight::Inflight;
pub(crate) use quorum_set::IdVal;
pub(crate) use quorum_set::VecProgress;
pub(crate) use quorum_set::VecProgressEntry;
pub(crate) use quorum_set::VecProgressEntryData;
