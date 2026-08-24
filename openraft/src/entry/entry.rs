use std::fmt;

use openraft_macros::since;

use crate::AppData;
use crate::EntryPayload;
use crate::Membership;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::log_id::LogId;
use crate::node::Node;
use crate::node::NodeId;
use crate::vote::RaftCommittedLeaderId;

/// A Raft log entry.
#[since(version = "0.10.0", change = "from `Entry<C>` to `Entry<CLID, D, NID, N>`")]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData,
    NID: NodeId,
    N: Node,
{
    /// The log ID uniquely identifying this entry.
    pub log_id: LogId<CLID>,

    /// This entry's payload.
    pub payload: EntryPayload<D, NID, N>,
}

impl<CLID, D, NID, N> Clone for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData + Clone,
    NID: NodeId,
    N: Node,
{
    fn clone(&self) -> Self {
        Self {
            log_id: self.log_id.clone(),
            payload: self.payload.clone(),
        }
    }
}

impl<CLID, D, NID, N> fmt::Debug for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", &self.payload).finish()
    }
}

impl<CLID, D, NID, N> PartialEq for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData + PartialEq,
    NID: NodeId,
    N: Node,
{
    fn eq(&self, other: &Self) -> bool {
        self.log_id == other.log_id && self.payload == other.payload
    }
}

impl<CLID, D, NID, N> AsRef<Entry<CLID, D, NID, N>> for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn as_ref(&self) -> &Entry<CLID, D, NID, N> {
        self
    }
}

impl<CLID, D, NID, N> fmt::Display for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.log_id, self.payload)
    }
}

impl<CLID, D, NID, N> RaftPayload<NID, N> for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn get_membership(&self) -> Option<Membership<NID, N>> {
        self.payload.get_membership()
    }
}

impl<CLID, D, NID, N> RaftEntry for Entry<CLID, D, NID, N>
where
    CLID: RaftCommittedLeaderId,
    D: AppData,
    NID: NodeId,
    N: Node,
{
    type CommittedLeaderId = CLID;
    type D = D;
    type NodeId = NID;
    type Node = N;

    fn new(log_id: LogId<CLID>, payload: EntryPayload<D, NID, N>) -> Self {
        Self { log_id, payload }
    }

    fn log_id_parts(&self) -> (&CLID, u64) {
        (&self.log_id.leader_id, self.log_id.index)
    }

    fn set_log_id(&mut self, new: LogId<CLID>) {
        self.log_id = new;
    }
}
