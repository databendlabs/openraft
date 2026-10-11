//! Entry payload types for log entries.

use std::fmt;
use std::fmt::Formatter;

use openraft_macros::since;

use crate::AppData;
use crate::membership::RaftMembership;

/// Log entry payload variants.
#[since(
    version = "0.10.0",
    change = "from `EntryPayload<C: RaftTypeConfig>` to `EntryPayload<D, Mem>`"
)]
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum EntryPayload<D, Mem>
where
    D: AppData,
    Mem: RaftMembership,
{
    /// An empty payload committed by a new cluster leader.
    Blank,

    /// Normal application data.
    Normal(D),

    /// A change-membership log entry.
    Membership(Mem),
}

impl<D, Mem> Clone for EntryPayload<D, Mem>
where
    D: AppData + Clone,
    Mem: RaftMembership,
{
    fn clone(&self) -> Self {
        match self {
            EntryPayload::Blank => EntryPayload::Blank,
            EntryPayload::Normal(n) => EntryPayload::Normal(n.clone()),
            EntryPayload::Membership(m) => EntryPayload::Membership(m.clone()),
        }
    }
}

impl<D, Mem> fmt::Debug for EntryPayload<D, Mem>
where
    D: AppData,
    Mem: RaftMembership,
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

impl<D, Mem> fmt::Display for EntryPayload<D, Mem>
where
    D: AppData,
    Mem: RaftMembership,
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

impl<D, Mem> EntryPayload<D, Mem>
where
    D: AppData,
    Mem: RaftMembership,
{
    pub fn type_str(&self) -> &'static str {
        match self {
            EntryPayload::Blank => "Blank",
            EntryPayload::Normal(_) => "Normal",
            EntryPayload::Membership(_) => "Membership",
        }
    }
}

impl<AppData, Mem> crate::entry::raft_payload::RaftPayload for EntryPayload<AppData, Mem>
where
    AppData: crate::AppData,
    Mem: RaftMembership,
{
    type D = AppData;
    type Membership = Mem;

    fn blank() -> Self {
        EntryPayload::Blank
    }

    fn with_normal(self, data: AppData) -> Self {
        EntryPayload::Normal(data)
    }

    fn with_membership(self, membership: Mem) -> Self {
        EntryPayload::Membership(membership)
    }

    fn get_membership(&self) -> Option<Mem> {
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

    use crate::entry::RaftPayload;
    use crate::entry::payload::EntryPayload;

    type TestPayload = EntryPayload<u64, crate::Membership<u64, ()>>;

    #[test]
    fn test_constructors() {
        let actual = TestPayload::blank();
        assert_eq!(actual, EntryPayload::Blank);

        let actual = TestPayload::normal(3);
        assert_eq!(actual, EntryPayload::Normal(3));

        let membership = crate::Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []);
        let actual = TestPayload::membership(membership.clone());
        assert_eq!(actual, EntryPayload::Membership(membership));
    }

    #[test]
    fn test_with_membership_replaces_normal() {
        let membership = crate::Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []);
        let payload = TestPayload::Normal(3);

        let actual = payload.with_membership(membership.clone());

        let expected = TestPayload::Membership(membership);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_normal_replaces_membership() {
        let membership = crate::Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []);
        let payload = TestPayload::Membership(membership);

        let actual = payload.with_normal(3);

        let expected = TestPayload::Normal(3);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_debug() {
        let blank = TestPayload::Blank;
        assert_eq!(format!("{:?}", blank), "blank");

        let normal = TestPayload::Normal(3);
        assert_eq!(format!("{:?}", normal), "normal:3");

        let membership =
            TestPayload::Membership(crate::Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []));
        assert_eq!(
            format!("{:?}", membership),
            "membership:Membership { configs: [{1, 2}], nodes: {1: (), 2: ()} }"
        );
    }

    #[test]
    fn test_display() {
        let blank = TestPayload::Blank;
        assert_eq!(format!("{}", blank), "blank");

        let normal = TestPayload::Normal(3);
        assert_eq!(format!("{}", normal), "normal:3");

        let membership =
            TestPayload::Membership(crate::Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []));
        assert_eq!(
            format!("{}", membership),
            "membership:{voters:[{1:(),2:()}], learners:[]}"
        );
    }
}
