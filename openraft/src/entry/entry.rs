use std::fmt;

use openraft_macros::since;

use crate::Membership;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::log_id::LogId;
use crate::vote::RaftCommittedLeaderId;

/// A Raft log entry.
#[since(version = "0.10.0", change = "from `Entry<CLID, D, NID, N>` to `Entry<CLID, P>`")]
#[since(version = "0.10.0", change = "from `Entry<C>` to `Entry<CLID, D, NID, N>`")]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload,
{
    /// The log ID uniquely identifying this entry.
    pub log_id: LogId<CLID>,

    /// This entry's payload.
    pub payload: P,
}

impl<CLID, P> Clone for Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload + Clone,
{
    fn clone(&self) -> Self {
        Self {
            log_id: self.log_id.clone(),
            payload: self.payload.clone(),
        }
    }
}

impl<CLID, P> fmt::Debug for Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", &self.payload).finish()
    }
}

impl<CLID, P> PartialEq for Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload + PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.log_id == other.log_id && self.payload == other.payload
    }
}

impl<CLID, P> AsRef<Entry<CLID, P>> for Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload,
{
    fn as_ref(&self) -> &Entry<CLID, P> {
        self
    }
}

impl<CLID, P> fmt::Display for Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.log_id, self.payload)
    }
}

impl<CLID, P> RaftEntry for Entry<CLID, P>
where
    CLID: RaftCommittedLeaderId,
    P: RaftPayload,
{
    type CommittedLeaderId = CLID;
    type Payload = P;

    fn new(log_id: LogId<CLID>, payload: P) -> Self {
        Self { log_id, payload }
    }

    fn log_id_parts(&self) -> (&CLID, u64) {
        (&self.log_id.leader_id, self.log_id.index)
    }

    fn set_log_id(&mut self, new: LogId<CLID>) {
        self.log_id = new;
    }

    fn get_membership(&self) -> Option<Membership<P::NodeId, P::Node>> {
        self.payload.get_membership()
    }
}
