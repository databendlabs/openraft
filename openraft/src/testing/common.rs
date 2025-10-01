//! Testing utilities used by all kinds of tests.

use std::collections::BTreeSet;

use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::type_config::alias::LogIdOf;
use crate::vote::RaftLeaderIdExt;

/// Builds a log id, for testing purposes.
pub fn log_id<C>(term: u64, node_id: C::NodeId, index: u64) -> LogIdOf<C>
where
    C: RaftTypeConfig,
    C::Term: From<u64>,
{
    LogIdOf::<C>::new(C::LeaderId::new_committed(term.into(), node_id), index)
}

/// Create a blank log entry for tests.
pub fn blank_ent<C>(term: u64, node_id: C::NodeId, index: u64) -> crate::Entry<C>
where
    C: RaftTypeConfig,
    C::Term: From<u64>,
{
    crate::Entry::<C>::new_blank(log_id::<C>(term, node_id, index))
}

/// Create a membership log entry without learner config for test.
pub fn membership_ent<C>(term: u64, node_id: C::NodeId, index: u64, config: Vec<BTreeSet<C::NodeId>>) -> crate::Entry<C>
where
    C: RaftTypeConfig,
    C::Term: From<u64>,
    C::Node: Default,
{
    crate::Entry::new_membership(
        log_id::<C>(term, node_id, index),
        crate::Membership::new_with_defaults(config, []),
    )
}
