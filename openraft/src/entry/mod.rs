use std::fmt::Debug;

use crate::raft_types::RaftLogId;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::RaftTypeConfig;

pub(crate) mod entry_ref;
pub mod payload;
mod traits;

pub(crate) use entry_ref::EntryRef;
pub use payload::EntryPayload;
pub use traits::RaftEntry;
pub use traits::RaftPayload;

/// A Raft log entry.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Entry<C>
where C: RaftTypeConfig
{
    pub log_id: LogId<C::NodeId>,

    /// This entry's payload.
    pub payload: EntryPayload<C>,
}

impl<C> Debug for Entry<C>
where
    C::D: Debug,
    C: RaftTypeConfig,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", &self.payload).finish()
    }
}

impl<C> Default for Entry<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self {
            log_id: LogId::default(),
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

impl<C> MessageSummary<Entry<C>> for Entry<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        format!("{}:{}", self.log_id, self.payload.summary())
    }
}

impl<C> From<&Entry<C>> for Entry<C>
where C: RaftTypeConfig
{
    fn from(er: &Entry<C>) -> Self {
        Entry {
            log_id: er.log_id,
            payload: er.payload.clone(),
        }
    }
}

impl<'p, C> From<&EntryRef<'p, C>> for Entry<C>
where C: RaftTypeConfig
{
    fn from(er: &EntryRef<'p, C>) -> Self {
        Entry {
            log_id: er.log_id,
            payload: er.payload.clone(),
        }
    }
}

impl<C> RaftPayload<C::NodeId, C::Node> for Entry<C>
where C: RaftTypeConfig
{
    fn is_blank(&self) -> bool {
        self.payload.is_blank()
    }

    fn get_membership(&self) -> Option<&Membership<C::NodeId, C::Node>> {
        self.payload.get_membership()
    }
}

impl<C> RaftLogId<C::NodeId> for Entry<C>
where C: RaftTypeConfig
{
    fn get_log_id(&self) -> &LogId<C::NodeId> {
        &self.log_id
    }

    fn set_log_id(&mut self, log_id: &LogId<C::NodeId>) {
        self.log_id = *log_id;
    }
}

impl<C> RaftEntry<C::NodeId, C::Node> for Entry<C> where C: RaftTypeConfig {}
