use std::fmt;
use std::fmt::Formatter;

use crate::entry::traits::RaftPayload;
use crate::Membership;
use crate::RaftTypeConfig;

/// Log entry payload variants.
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum EntryPayload<C: RaftTypeConfig> {
    /// An empty payload committed by a new cluster leader.
    Blank,

    Normal(C::D),

    /// A change-membership log entry.
    Membership(Membership<C>),
}

impl<C> Clone for EntryPayload<C>
where
    C: RaftTypeConfig,
    C::D: Clone,
{
    fn clone(&self) -> Self {
        match self {
            EntryPayload::Blank => EntryPayload::Blank,
            EntryPayload::Normal(n) => EntryPayload::Normal(n.clone()),
            EntryPayload::Membership(m) => EntryPayload::Membership(m.clone()),
        }
    }
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

impl<C> fmt::Display for EntryPayload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EntryPayload::Blank => write!(f, "blank")?,
            EntryPayload::Normal(_n) => write!(f, "normal")?,
            EntryPayload::Membership(c) => {
                write!(f, "membership:{}", c)?;
            }
        }

        Ok(())
    }
}

impl<C: RaftTypeConfig> RaftPayload<C> for EntryPayload<C> {
    fn get_membership(&self) -> Option<Membership<C>> {
        if let EntryPayload::Membership(m) = self {
            Some(m.clone())
        } else {
            None
        }
    }
}
