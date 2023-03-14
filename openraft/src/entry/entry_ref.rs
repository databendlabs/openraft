use std::fmt::Debug;

use crate::entry::traits::RaftPayload;
use crate::entry::RaftEntry;
use crate::log_id::RaftLogId;
use crate::EntryPayload;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::RaftTypeConfig;

/// A Raft log entry that does not own its payload.
///
/// This is only used internally, to avoid memory copy for the payload.
#[derive(Clone)]
pub(crate) struct EntryRef<'p, C: RaftTypeConfig> {
    pub log_id: LogId<C::NodeId>,
    pub payload: &'p EntryPayload<C>,
}

impl<'p, C> Debug for EntryRef<'p, C>
where
    C::D: Debug,
    C: RaftTypeConfig,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", self.payload).finish()
    }
}

impl<'p, C> MessageSummary<EntryRef<'p, C>> for EntryRef<'p, C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        format!("{}:{}", self.log_id, self.payload.summary())
    }
}

impl<'p, C> EntryRef<'p, C>
where C: RaftTypeConfig
{
    pub fn new(payload: &'p EntryPayload<C>) -> Self {
        Self {
            log_id: Default::default(),
            payload,
        }
    }
}

impl<'p, C> RaftPayload<C::NodeId, C::Node> for EntryRef<'p, C>
where C: RaftTypeConfig
{
    fn is_blank(&self) -> bool {
        self.payload.is_blank()
    }

    fn get_membership(&self) -> Option<&Membership<C::NodeId, C::Node>> {
        self.payload.get_membership()
    }
}

impl<'p, C> RaftLogId<C::NodeId> for EntryRef<'p, C>
where C: RaftTypeConfig
{
    fn get_log_id(&self) -> &LogId<C::NodeId> {
        &self.log_id
    }

    fn set_log_id(&mut self, log_id: &LogId<C::NodeId>) {
        self.log_id = *log_id;
    }
}

impl<'p, C> RaftEntry<C::NodeId, C::Node> for EntryRef<'p, C> where C: RaftTypeConfig {}
