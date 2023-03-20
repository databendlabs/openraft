use std::fmt;
use std::fmt::Formatter;

use crate::entry::traits::RaftPayload;
use crate::Membership;
use crate::MessageSummary;
use crate::RaftTypeConfig;

/// Log entry payload variants.
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum EntryPayload<C: RaftTypeConfig> {
    /// An empty payload committed by a new cluster leader.
    Blank,

    Normal(C::D),

    /// A change-membership log entry.
    Membership(Membership<C::NodeId, C::Node>),
}

impl<C: RaftTypeConfig> fmt::Debug for EntryPayload<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EntryPayload::Blank => write!(f, "blank")?,
            EntryPayload::Normal(_n) => write!(f, "normal")?,
            EntryPayload::Membership(c) => {
                write!(f, "membership:{:?}", c)?;
            }
        }

        Ok(())
    }
}

impl<C: RaftTypeConfig> MessageSummary<EntryPayload<C>> for EntryPayload<C> {
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

impl<C: RaftTypeConfig> RaftPayload<C::NodeId, C::Node> for EntryPayload<C> {
    fn is_blank(&self) -> bool {
        matches!(self, EntryPayload::Blank)
    }

    fn get_membership(&self) -> Option<&Membership<C::NodeId, C::Node>> {
        if let EntryPayload::Membership(m) = self {
            Some(m)
        } else {
            None
        }
    }
}
