use std::fmt;

use openraft_macros::since;

use crate::base::OptionalSend;
use crate::membership::RaftMembership;
use crate::storage::SnapshotMeta;
use crate::vote::RaftCommittedLeaderId;

/// The data associated with the current snapshot.
#[since(version = "0.10.0", change = "from `Snapshot<C>` to `Snapshot<CLID, Mem, SD>`")]
#[since(version = "0.10.0", change = "SnapshotData without Box")]
#[derive(Debug, Clone)]
pub struct Snapshot<CLID, Mem, SD>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
    SD: OptionalSend + 'static,
{
    /// metadata of a snapshot
    pub meta: SnapshotMeta<CLID, Mem>,

    /// A read handle to the associated snapshot.
    pub snapshot: SD,
}

impl<CLID, Mem, SD> Snapshot<CLID, Mem, SD>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
    SD: OptionalSend + 'static,
{
    #[allow(dead_code)]
    pub(crate) fn new(meta: SnapshotMeta<CLID, Mem>, snapshot: SD) -> Self {
        Self { meta, snapshot }
    }
}

impl<CLID, Mem, SD> fmt::Display for Snapshot<CLID, Mem, SD>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
    SD: OptionalSend + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Snapshot{{meta: {}}}", self.meta)
    }
}
