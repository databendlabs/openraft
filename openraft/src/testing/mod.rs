//! Testing utilities for OpenRaft.

mod store_builder;
mod suite;

use std::collections::BTreeSet;

pub use store_builder::StoreBuilder;
pub use suite::Suite;

use crate::entry::RaftEntry;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftTypeConfig;

/// Builds a log id, for testing purposes.
pub fn log_id<NID: crate::NodeId>(term: u64, node_id: NID, index: u64) -> LogId<NID> {
    LogId::<NID> {
        leader_id: CommittedLeaderId::new(term, node_id),
        index,
    }
}

/// Create a blank log entry for test.
pub fn blank_ent<C: RaftTypeConfig>(term: u64, node_id: C::NodeId, index: u64) -> crate::Entry<C> {
    crate::Entry::<C>::new_blank(LogId::new(CommittedLeaderId::new(term, node_id), index))
}

/// Create a membership log entry without learner config for test.
pub fn membership_ent<C: RaftTypeConfig>(
    term: u64,
    node_id: C::NodeId,
    index: u64,
    config: Vec<BTreeSet<C::NodeId>>,
) -> crate::Entry<C> {
    crate::Entry::new_membership(
        LogId::new(CommittedLeaderId::new(term, node_id), index),
        crate::Membership::new(config, None),
    )
}
