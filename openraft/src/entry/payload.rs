//! Entry payload types for log entries.

use std::fmt;
use std::fmt::Formatter;

use crate::Membership;
use crate::RaftTypeConfig;
use crate::entry::raft_payload::RaftPayload;

/// Log entry payload variants.
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum EntryPayload<C: RaftTypeConfig> {
    /// An empty payload committed by a new cluster leader.
    Blank,

    /// Normal application data.
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

impl<C> fmt::Debug for EntryPayload<C>
where C: RaftTypeConfig
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

impl<C> fmt::Display for EntryPayload<C>
where C: RaftTypeConfig
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

impl<C> EntryPayload<C>
where C: RaftTypeConfig
{
    pub fn type_str(&self) -> &'static str {
        match self {
            EntryPayload::Blank => "Blank",
            EntryPayload::Normal(_) => "Normal",
            EntryPayload::Membership(_) => "Membership",
        }
    }
}

impl<C> RaftPayload<C> for EntryPayload<C>
where C: RaftTypeConfig
{
    fn get_membership(&self) -> Option<Membership<C>> {
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

    use crate::engine::testing::UTConfig;
    use crate::entry::payload::EntryPayload;

    #[test]
    fn test_debug() {
        let blank = EntryPayload::<UTConfig>::Blank;
        assert_eq!(format!("{:?}", blank), "blank");

        let normal = EntryPayload::<UTConfig>::Normal(3);
        assert_eq!(format!("{:?}", normal), "normal:3");

        let membership = EntryPayload::<UTConfig>::Membership(crate::Membership::new_with_defaults(
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
        let blank = EntryPayload::<UTConfig>::Blank;
        assert_eq!(format!("{}", blank), "blank");

        let normal = EntryPayload::<UTConfig>::Normal(3);
        assert_eq!(format!("{}", normal), "normal:3");

        let membership = EntryPayload::<UTConfig>::Membership(crate::Membership::new_with_defaults(
            vec![BTreeSet::from([1, 2])],
            [],
        ));
        assert_eq!(
            format!("{}", membership),
            "membership:{voters:[{1:(),2:()}], learners:[]}"
        );
    }
}
