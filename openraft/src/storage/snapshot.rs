use std::fmt;

use openraft_macros::since;

use crate::MembershipMetadata;
use crate::base::OptionalSend;
use crate::node::Node;
use crate::node::NodeId;
use crate::storage::SnapshotMeta;
use crate::vote::RaftCommittedLeaderId;

/// The data associated with the current snapshot.
#[since(version = "0.10.0", change = "added membership-wide metadata type `M`")]
#[since(version = "0.10.0", change = "from `Snapshot<C>` to `Snapshot<CLID, NID, N, SD>`")]
#[since(version = "0.10.0", change = "SnapshotData without Box")]
#[derive(Debug, Clone)]
pub struct Snapshot<CLID, NID, N, SD, M = ()>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    SD: OptionalSend + 'static,
    M: MembershipMetadata,
{
    /// metadata of a snapshot
    pub meta: SnapshotMeta<CLID, NID, N, M>,

    /// A read handle to the associated snapshot.
    pub snapshot: SD,
}

impl<CLID, NID, N, SD, M> Snapshot<CLID, NID, N, SD, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    SD: OptionalSend + 'static,
    M: MembershipMetadata,
{
    #[allow(dead_code)]
    pub(crate) fn new(meta: SnapshotMeta<CLID, NID, N, M>, snapshot: SD) -> Self {
        Self { meta, snapshot }
    }
}

impl<CLID, NID, N, SD, M> fmt::Display for Snapshot<CLID, NID, N, SD, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    SD: OptionalSend + 'static,
    M: MembershipMetadata,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Snapshot{{meta: {}}}", self.meta)
    }
}
