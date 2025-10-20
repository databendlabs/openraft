use std::fmt;

use crate::EntryPayload;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// A Raft log entry.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Entry<C>
where C: RaftTypeConfig
{
    /// The log ID uniquely identifying this entry.
    pub log_id: LogIdOf<C>,

    /// This entry's payload.
    pub payload: EntryPayload<C>,
}

impl<C> Clone for Entry<C>
where
    C: RaftTypeConfig,
    C::D: Clone,
{
    fn clone(&self) -> Self {
        Self {
            log_id: self.log_id.clone(),
            payload: self.payload.clone(),
        }
    }
}

impl<C> fmt::Debug for Entry<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", &self.payload).finish()
    }
}

impl<C> Default for Entry<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self {
            log_id: LogIdOf::<C>::default(),
            payload: EntryPayload::Blank,
        }
    }
}

impl<C> PartialEq for Entry<C>
where
    C::D: PartialEq,
    C: RaftTypeConfig,
{
    fn eq(&self, other: &Self) -> bool {
        self.log_id == other.log_id && self.payload == other.payload
    }
}

impl<C> AsRef<Entry<C>> for Entry<C>
where C: RaftTypeConfig
{
    fn as_ref(&self) -> &Entry<C> {
        self
    }
}

impl<C> fmt::Display for Entry<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.log_id, self.payload)
    }
}

impl<C> RaftPayload<C> for Entry<C>
where C: RaftTypeConfig
{
    fn get_membership(&self) -> Option<Membership<C>> {
        self.payload.get_membership()
    }
}

impl<C> RaftEntry<C> for Entry<C>
where C: RaftTypeConfig
{
    fn new(log_id: LogIdOf<C>, payload: EntryPayload<C>) -> Self {
        Self { log_id, payload }
    }

    fn log_id_parts(&self) -> (&CommittedLeaderIdOf<C>, u64) {
        (&self.log_id.leader_id, self.log_id.index)
    }

    fn set_log_id(&mut self, new: LogIdOf<C>) {
        self.log_id = new;
    }
}
