use crate::entry::RaftEntry;
use crate::testing::log_id1;
use crate::RaftTypeConfig;

/// Trivial Raft type config for Engine related unit test.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct UTConfig {}
impl RaftTypeConfig for UTConfig {
    type D = ();
    type R = ();
    type NodeId = u64;
    type Node = ();
    type Entry = crate::Entry<UTConfig>;
}

pub(crate) fn blank_ent(term: u64, index: u64) -> crate::Entry<UTConfig> {
    crate::Entry::<UTConfig>::new_blank(log_id1(term, index))
}
