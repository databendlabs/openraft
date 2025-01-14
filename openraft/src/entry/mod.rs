//! The default log entry type that implements [`RaftEntry`].

use std::fmt;
use std::fmt::Debug;

use crate::Membership;
use crate::RaftTypeConfig;

pub mod payload;
mod traits;

pub use payload::EntryPayload;
pub use traits::RaftEntry;
pub use traits::RaftEntryExt;
pub use traits::RaftPayload;

use crate::type_config::alias::AppDataOf;
use crate::type_config::alias::LogIdOf;

/// A Raft log entry.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Entry<C>
where C: RaftTypeConfig
{
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

impl<C> Debug for Entry<C>
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
    fn is_blank(&self) -> bool {
        self.payload.is_blank()
    }

    fn get_membership(&self) -> Option<&Membership<C>> {
        self.payload.get_membership()
    }
}

impl<C> RaftEntry<C> for Entry<C>
where C: RaftTypeConfig
{
    fn new_blank(log_id: LogIdOf<C>) -> Self {
        Self {
            log_id,
            payload: EntryPayload::Blank,
        }
    }

    fn new_normal(log_id: LogIdOf<C>, data: AppDataOf<C>) -> Self {
        Entry {
            log_id,
            payload: EntryPayload::Normal(data),
        }
    }

    fn new_membership(log_id: LogIdOf<C>, m: Membership<C>) -> Self {
        Self {
            log_id,
            payload: EntryPayload::Membership(m),
        }
    }

    fn log_id(&self) -> &LogIdOf<C> {
        todo!()
    }

    fn set_log_id(&mut self, new: &LogIdOf<C>) {
        todo!()
    }
}
