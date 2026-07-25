//! Progress tracks replication state, i.e., it can be considered a map of node id to already
//! replicated log id.
//!
//! The "progress" internally is a vector of scalar values.
//! The scalar value is monotonically incremental. Decreasing it is not allowed.
//! Optimization on calculating the quorum-accepted log id is done on this assumption.

#[cfg(feature = "bench")]
#[cfg(test)]
mod bench;
pub(crate) mod display_vec_progress;
pub(crate) mod entry;
pub(crate) mod id_val;
pub(crate) mod inflight;
pub(crate) mod inflight_id;
pub(crate) mod progress_stats;
pub(crate) mod stream_id;
pub(crate) mod vec_progress;
pub(crate) mod vec_progress_entry;

// TODO: remove it
#[allow(unused_imports)]
pub(crate) use inflight::Inflight;
pub(crate) use vec_progress::VecProgress;
pub(crate) use vec_progress_entry::VecProgressEntry;
pub(crate) use vec_progress_entry::VecProgressEntryData;
