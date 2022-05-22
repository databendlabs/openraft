use std::fmt::Debug;

use crate::raft_types::RaftLogId;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftTypeConfig;

/// Defines operations on an entry payload.
pub trait RaftPayload<NID: NodeId> {
    /// Return `Some(())` if the entry payload is blank.
    fn is_blank(&self) -> bool;

    /// Return `Some(&Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<&Membership<NID>>;
}

/// Defines operations on an entry.
pub trait RaftEntry<NID: NodeId>: RaftPayload<NID> + RaftLogId<NID> {}

/// Log entry payload variants.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum EntryPayload<C: RaftTypeConfig> {
    /// An empty payload committed by a new cluster leader.
    Blank,

    Normal(C::D),

    /// A change-membership log entry.
    Membership(Membership<C::NodeId>),
}

impl<C: RaftTypeConfig> MessageSummary for EntryPayload<C> {
    fn summary(&self) -> String {
        match self {
            EntryPayload::Blank => "blank".to_string(),
            EntryPayload::Normal(_n) => "normal".to_string(),
            EntryPayload::Membership(c) => {
                format!("membership: {}", c.summary())
            }
        }
    }
}

/// A Raft log entry.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Entry<C: RaftTypeConfig> {
    pub log_id: LogId<C::NodeId>,

    /// This entry's payload.
    pub payload: EntryPayload<C>,
}

impl<C: RaftTypeConfig> Debug for Entry<C>
where C::D: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", &self.payload).finish()
    }
}

impl<C: RaftTypeConfig> Default for Entry<C> {
    fn default() -> Self {
        Self {
            log_id: LogId::default(),
            payload: EntryPayload::Blank,
        }
    }
}

impl<C: RaftTypeConfig> MessageSummary for Entry<C> {
    fn summary(&self) -> String {
        format!("{}:{}", self.log_id, self.payload.summary())
    }
}

impl<C: RaftTypeConfig> MessageSummary for Option<Entry<C>> {
    fn summary(&self) -> String {
        match self {
            None => "None".to_string(),
            Some(x) => format!("Some({})", x.summary()),
        }
    }
}

impl<C: RaftTypeConfig> MessageSummary for &[Entry<C>] {
    fn summary(&self) -> String {
        let entry_refs: Vec<_> = self.iter().collect();
        entry_refs.as_slice().summary()
    }
}

impl<C: RaftTypeConfig> MessageSummary for &[&Entry<C>] {
    fn summary(&self) -> String {
        if self.is_empty() {
            return "{}".to_string();
        }
        let mut res = Vec::with_capacity(self.len());
        if self.len() <= 5 {
            for x in self.iter() {
                let e = format!("{}:{}", x.log_id, x.payload.summary());
                res.push(e);
            }

            res.join(",")
        } else {
            let first = *self.first().unwrap();
            let last = *self.last().unwrap();

            format!("{} ... {}", first.summary(), last.summary())
        }
    }
}

/// A Raft log entry that does not own its payload.
///
/// This is only used internally, to avoid memory copy for the payload.
#[derive(Clone)]
pub(crate) struct EntryRef<'p, C: RaftTypeConfig> {
    pub log_id: LogId<C::NodeId>,
    pub payload: &'p EntryPayload<C>,
}

impl<'p, C: RaftTypeConfig> Debug for EntryRef<'p, C>
where C::D: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("log_id", &self.log_id).field("payload", self.payload).finish()
    }
}

impl<'p, C: RaftTypeConfig> MessageSummary for EntryRef<'p, C> {
    fn summary(&self) -> String {
        format!("{}:{}", self.log_id, self.payload.summary())
    }
}

impl<'p, C: RaftTypeConfig> From<&EntryRef<'p, C>> for Entry<C> {
    fn from(er: &EntryRef<'p, C>) -> Self {
        Entry {
            log_id: er.log_id,
            payload: er.payload.clone(),
        }
    }
}
impl<'p, C: RaftTypeConfig> EntryRef<'p, C> {
    pub fn new(payload: &'p EntryPayload<C>) -> Self {
        Self {
            log_id: Default::default(),
            payload,
        }
    }
}

// impl traits for EntryPayload

impl<C: RaftTypeConfig> RaftPayload<C::NodeId> for EntryPayload<C> {
    fn is_blank(&self) -> bool {
        matches!(self, EntryPayload::Blank)
    }

    fn get_membership(&self) -> Option<&Membership<C::NodeId>> {
        if let EntryPayload::Membership(m) = self {
            Some(m)
        } else {
            None
        }
    }
}

// impl traits for Entry

impl<C: RaftTypeConfig> RaftPayload<C::NodeId> for Entry<C> {
    fn is_blank(&self) -> bool {
        self.payload.is_blank()
    }

    fn get_membership(&self) -> Option<&Membership<C::NodeId>> {
        self.payload.get_membership()
    }
}

impl<C: RaftTypeConfig> RaftLogId<C::NodeId> for Entry<C> {
    fn get_log_id(&self) -> &LogId<C::NodeId> {
        &self.log_id
    }

    fn set_log_id(&mut self, log_id: &LogId<C::NodeId>) {
        self.log_id = *log_id;
    }
}

impl<C: RaftTypeConfig> RaftEntry<C::NodeId> for Entry<C> {}

// impl traits for RefEntry

impl<'p, C: RaftTypeConfig> RaftPayload<C::NodeId> for EntryRef<'p, C> {
    fn is_blank(&self) -> bool {
        self.payload.is_blank()
    }

    fn get_membership(&self) -> Option<&Membership<C::NodeId>> {
        self.payload.get_membership()
    }
}

impl<'p, C: RaftTypeConfig> RaftLogId<C::NodeId> for EntryRef<'p, C> {
    fn get_log_id(&self) -> &LogId<C::NodeId> {
        &self.log_id
    }

    fn set_log_id(&mut self, log_id: &LogId<C::NodeId>) {
        self.log_id = *log_id;
    }
}

impl<'p, C: RaftTypeConfig> RaftEntry<C::NodeId> for EntryRef<'p, C> {}
