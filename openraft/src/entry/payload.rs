//! Entry payload types for log entries.

use std::fmt;
use std::fmt::Formatter;

use openraft_macros::since;

use crate::AppData;
use crate::Membership;
use crate::node::Node;
use crate::node::NodeId;

/// Log entry payload variants.
#[since(
    version = "0.10.0",
    change = "from `EntryPayload<C: RaftTypeConfig>` to `EntryPayload<D, NID, N>`"
)]
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    /// An empty payload committed by a new cluster leader.
    Blank,

    /// Normal application data.
    Normal(D),

    /// A change-membership log entry.
    Membership(Membership<NID, N>),
}

impl<D, NID, N> Clone for EntryPayload<D, NID, N>
where
    D: AppData + Clone,
    NID: NodeId,
    N: Node,
{
    fn clone(&self) -> Self {
        match self {
            EntryPayload::Blank => EntryPayload::Blank,
            EntryPayload::Normal(n) => EntryPayload::Normal(n.clone()),
            EntryPayload::Membership(m) => EntryPayload::Membership(m.clone()),
        }
    }
}

impl<D, NID, N> fmt::Debug for EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EntryPayload::Blank => write!(f, "blank")?,
            EntryPayload::Normal(app_data) => write!(f, "normal:{:?}", app_data)?,
            EntryPayload::Membership(c) => {
                write!(f, "membership:{:?}", c)?;
            }
        }

        Ok(())
    }
}

impl<D, NID, N> fmt::Display for EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            EntryPayload::Blank => write!(f, "blank")?,
            EntryPayload::Normal(app_data) => write!(f, "normal:{}", app_data)?,
            EntryPayload::Membership(c) => {
                write!(f, "membership:{}", c)?;
            }
        }

        Ok(())
    }
}

impl<D, NID, N> EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    pub fn type_str(&self) -> &'static str {
        match self {
            EntryPayload::Blank => "Blank",
            EntryPayload::Normal(_) => "Normal",
            EntryPayload::Membership(_) => "Membership",
        }
    }
}

impl<D, NID, N> crate::entry::raft_payload::RaftPayload<NID, N> for EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn get_membership(&self) -> Option<Membership<NID, N>> {
        if let EntryPayload::Membership(m) = self {
            Some(m.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::entry::payload::EntryPayload;

    #[test]
    fn test_debug() {
        let blank = EntryPayload::<u64, u64, ()>::Blank;
        assert_eq!(format!("{:?}", blank), "blank");

        let normal = EntryPayload::<u64, u64, ()>::Normal(3);
        assert_eq!(format!("{:?}", normal), "normal:3");

        let membership = EntryPayload::<u64, u64, ()>::Membership(crate::Membership::new_with_defaults(
            vec![BTreeSet::from([1, 2])],
            [],
        ));
        assert_eq!(
            format!("{:?}", membership),
            "membership:Membership { configs: [{1, 2}], nodes: {1: (), 2: ()} }"
        );
    }

    #[test]
    fn test_display() {
        let blank = EntryPayload::<u64, u64, ()>::Blank;
        assert_eq!(format!("{}", blank), "blank");

        let normal = EntryPayload::<u64, u64, ()>::Normal(3);
        assert_eq!(format!("{}", normal), "normal:3");

        let membership = EntryPayload::<u64, u64, ()>::Membership(crate::Membership::new_with_defaults(
            vec![BTreeSet::from([1, 2])],
            [],
        ));
        assert_eq!(
            format!("{}", membership),
            "membership:{voters:[{1:(),2:()}], learners:[]}"
        );
    }
}
