//! Per-node progress tracking, and the leader's replication state for each target.
//!
//! A [`VecProgress`] keeps an ordered progress value for each node and the greatest value accepted
//! by a quorum, the quorum-accepted value. A candidate tracks vote grants in one, as `bool` values.
//! A leader keeps two: one holds each node's [`ProgressEntry`], whose progress value is the
//! matching log id, and the other holds the [`SendStamp`] of the latest RPC that each node
//! acknowledged.
//!
//! The ordinary updates, [`VecProgress::update_entry_with()`], [`VecProgress::update_progress()`]
//! and [`VecProgress::increase_to()`], must keep or increase a value. `VecProgress` relies on this
//! to avoid most sorting and quorum recalculation.
//!
//! [`VecProgress::reset_entry_with()`] is the only update that may lower a value. It moves the
//! entry down to restore the voter ordering, but keeps the quorum-accepted value, because a value
//! that a quorum accepted is never withdrawn. The leader lowers a value only when a conflict shows
//! that a follower's log reverted and log reversion is allowed: [`Updater::update_conflicting()`]
//! then resets the follower's matching log id to `None`.
//!
//! When membership changes, the leader rebuilds both of its trackers for the new quorum set with
//! [`VecProgress::upgrade_quorum_set()`]. Nodes that remain keep their entries, and the
//! quorum-accepted value is computed again, so it may decrease.
//!
//! [`VecProgress::iter_mut_without_reorder()`] does not restore the ordering: use it to change an
//! entry's other fields, never its progress value.
//!
//! [`ProgressEntry`]: crate::progress::entry::ProgressEntry
//! [`SendStamp`]: crate::proposer::SendStamp
//! [`Updater::update_conflicting()`]: crate::progress::entry::update::Updater::update_conflicting

pub(crate) mod entry;
pub(crate) mod inflight;
pub(crate) mod inflight_id;
pub(crate) mod stream_id;

pub(crate) use inflight::Inflight;
pub(crate) use quorum_set::IdVal;
pub(crate) use quorum_set::VecProgress;
pub(crate) use quorum_set::VecProgressEntry;
pub(crate) use quorum_set::VecProgressEntryData;
