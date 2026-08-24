//! Entry payload types for log entries.

use std::fmt;
use std::fmt::Formatter;

use openraft_macros::since;

use crate::AppData;
use crate::Membership;
use crate::node::Node;
use crate::node::NodeId;

/// The payload of a Raft log entry.
///
/// Application data and a membership configuration are independent so that one log entry can
/// atomically carry both. If both fields are `None`, this is a blank entry.
#[since(
    version = "0.10.0",
    change = "changed from `EntryPayload<C: RaftTypeConfig>` enum to `EntryPayload<D, NID, N>` struct with independent optional fields"
)]
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    /// Normal application data to apply to the state machine.
    #[since(version = "0.10.0")]
    pub normal: Option<D>,

    /// A membership configuration to apply at the same log index.
    #[since(version = "0.10.0")]
    pub membership: Option<Membership<NID, N>>,
}

impl<D, NID, N> Clone for EntryPayload<D, NID, N>
where
    D: AppData + Clone,
    NID: NodeId,
    N: Node,
{
    fn clone(&self) -> Self {
        Self {
            normal: self.normal.clone(),
            membership: self.membership.clone(),
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
        match (&self.normal, &self.membership) {
            (None, None) => write!(f, "blank"),
            (Some(app_data), None) => write!(f, "normal:{app_data:?}"),
            (None, Some(membership)) => write!(f, "membership:{membership:?}"),
            (Some(app_data), Some(membership)) => {
                write!(f, "normal:{app_data:?},membership:{membership:?}")
            }
        }
    }
}

impl<D, NID, N> fmt::Display for EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match (&self.normal, &self.membership) {
            (None, None) => write!(f, "blank"),
            (Some(app_data), None) => write!(f, "normal:{app_data}"),
            (None, Some(membership)) => write!(f, "membership:{membership}"),
            (Some(app_data), Some(membership)) => {
                write!(f, "normal:{app_data},membership:{membership}")
            }
        }
    }
}

impl<D, NID, N> EntryPayload<D, NID, N>
where
    D: AppData,
    NID: NodeId,
    N: Node,
{
    /// Create a payload from its independent application-data and membership parts.
    #[since(version = "0.10.0")]
    pub fn new(normal: Option<D>, membership: Option<Membership<NID, N>>) -> Self {
        Self { normal, membership }
    }

    /// Create a blank payload.
    #[since(version = "0.10.0")]
    pub fn blank() -> Self {
        Self::new(None, None)
    }

    /// Return whether this payload contains neither application data nor a membership configuration.
    #[since(version = "0.10.0")]
    pub fn is_empty(&self) -> bool {
        self.normal.is_none() && self.membership.is_none()
    }

    /// Create a payload containing only application data.
    #[since(version = "0.10.0")]
    pub fn normal(data: D) -> Self {
        Self::new(Some(data), None)
    }

    /// Create a payload containing only a membership configuration.
    #[since(version = "0.10.0")]
    pub fn membership(membership: Membership<NID, N>) -> Self {
        Self::new(None, Some(membership))
    }

    /// Return a short description of the fields present in this payload.
    pub fn type_str(&self) -> &'static str {
        match (&self.normal, &self.membership) {
            (None, None) => "Blank",
            (Some(_), None) => "Normal",
            (None, Some(_)) => "Membership",
            (Some(_), Some(_)) => "Normal+Membership",
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
        self.membership.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::entry::payload::EntryPayload;

    #[test]
    fn test_debug() {
        let blank = EntryPayload::<u64, u64, ()>::blank();
        assert_eq!(format!("{:?}", blank), "blank");

        let normal = EntryPayload::<u64, u64, ()>::normal(3);
        assert_eq!(format!("{:?}", normal), "normal:3");

        let membership = EntryPayload::<u64, u64, ()>::membership(crate::Membership::new_with_defaults(
            vec![BTreeSet::from([1, 2])],
            [],
        ));
        assert_eq!(
            format!("{:?}", membership),
            "membership:Membership { configs: [{1, 2}], nodes: {1: (), 2: ()} }"
        );

        let both = EntryPayload::new(Some(3), membership.membership);
        assert_eq!(
            format!("{:?}", both),
            "normal:3,membership:Membership { configs: [{1, 2}], nodes: {1: (), 2: ()} }"
        );
    }

    #[test]
    fn test_is_empty() {
        let blank = EntryPayload::<u64, u64, ()>::blank();
        assert!(blank.is_empty());

        let normal = EntryPayload::<u64, u64, ()>::normal(3);
        assert!(!normal.is_empty());

        let membership = EntryPayload::<u64, u64, ()>::membership(crate::Membership::new_with_defaults(
            vec![BTreeSet::from([1, 2])],
            [],
        ));
        assert!(!membership.is_empty());

        let both = EntryPayload::new(Some(3), membership.membership);
        assert!(!both.is_empty());
    }

    #[test]
    fn test_display() {
        let blank = EntryPayload::<u64, u64, ()>::blank();
        assert_eq!(format!("{}", blank), "blank");

        let normal = EntryPayload::<u64, u64, ()>::normal(3);
        assert_eq!(format!("{}", normal), "normal:3");

        let membership = EntryPayload::<u64, u64, ()>::membership(crate::Membership::new_with_defaults(
            vec![BTreeSet::from([1, 2])],
            [],
        ));
        assert_eq!(
            format!("{}", membership),
            "membership:{voters:[{1:(),2:()}], learners:[]}"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serde_with_normal_and_membership() -> anyhow::Result<()> {
        let payload = EntryPayload::new(
            Some(3_u64),
            Some(crate::Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], [])),
        );

        let json = serde_json::to_string(&payload)?;
        let restored: EntryPayload<u64, u64, ()> = serde_json::from_str(&json)?;

        assert_eq!(payload, restored);
        assert!(restored.normal.is_some());
        assert!(restored.membership.is_some());
        Ok(())
    }
}
