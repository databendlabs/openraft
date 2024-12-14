//! Testing utilities used by all kinds of tests.

use std::collections::BTreeSet;

use crate::entry::RaftEntry;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftTypeConfig;

/// Builds a log id, for testing purposes.
pub fn log_id<C>(term: u64, node_id: C::NodeId, index: u64) -> LogId<C>
where
    C: RaftTypeConfig,
    C::Term: From<u64>,
{
    LogId::<C> {
        leader_id: CommittedLeaderId::new(term.into(), node_id),
        index,
    }
}

/// Create a blank log entry for test.
pub fn blank_ent<C>(term: u64, node_id: C::NodeId, index: u64) -> crate::Entry<C>
where
    C: RaftTypeConfig,
    C::Term: From<u64>,
{
    crate::Entry::<C>::new_blank(log_id(term, node_id, index))
}

/// Create a membership log entry without learner config for test.
pub fn membership_ent<C: RaftTypeConfig>(
    term: u64,
    node_id: C::NodeId,
    index: u64,
    config: Vec<BTreeSet<C::NodeId>>,
) -> crate::Entry<C>
where
    C::Term: From<u64>,
    C::Node: Default,
{
    crate::Entry::new_membership(
        log_id(term, node_id, index),
        crate::Membership::new_with_defaults(config, []),
    )
}
