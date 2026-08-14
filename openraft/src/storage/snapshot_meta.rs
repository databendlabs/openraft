use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::MembershipMetadata;
use crate::StoredMembership;
use crate::log_id::LogId;
use crate::node::Node;
use crate::node::NodeId;
use crate::storage::SnapshotSignature;
use crate::vote::RaftCommittedLeaderId;

/// The metadata of a snapshot.
///
/// Including the last log id that is included in this snapshot
/// and the last membership included.
///
/// # Compatibility with 0.9
///
/// Before 0.10.0 this type also carried a `snapshot_id`, declared last. It identified a transfer,
/// not the snapshot: two snapshots at the same `last_log_id` cover the same state, even when they
/// differ in bytes. The id now lives on the wire, in
/// `openraft_legacy::network_v1::SnapshotMeta`, the metadata type of the chunked v1 protocol,
/// which keeps the full 0.9 layout.
///
/// Dropping it from the serialized form as well would have broken stored 0.9 data. A positional
/// format (`bincode`, `postcard`, `rmp-serde::to_vec`) encodes a struct as a bare sequence with no
/// field names, so two fields cannot be read off a three-element record. Worse, when this type is
/// nested in a larger struct, some of those formats misparse it *silently*: the leftover
/// `snapshot_id` bytes get consumed as the enclosing struct's next field, with no error.
///
/// So the slot is reserved rather than removed. This type serializes as three fields, the third
/// being an always-empty `snapshot_id`, and ignores that field when reading. 0.9 and 0.10 data are
/// therefore interchangeable in both directions, under named and positional formats alike.
#[since(version = "0.10.0", change = "added membership-wide metadata type `M`")]
#[since(version = "0.10.0", change = "removed `snapshot_id`")]
#[since(
    version = "0.10.0",
    change = "from `SnapshotMeta<C>` to `SnapshotMeta<CLID, NID, N>`"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize),
    serde(bound = "", from = "SnapshotMetaWire<CLID, NID, N, M>")
)]
pub struct SnapshotMeta<CLID, NID, N, M = ()>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    /// Log entries up to which this snapshot includes, inclusive.
    pub last_log_id: Option<LogId<CLID>>,

    /// The last applied membership config.
    pub last_membership: StoredMembership<CLID, NID, N, M>,
}

/// The serialized layout of [`SnapshotMeta`], which still carries the 0.9 `snapshot_id` slot.
///
/// Written always-empty and ignored on read; see the compatibility note on [`SnapshotMeta`].
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(bound = "")]
struct SnapshotMetaWire<CLID, NID, N, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    last_log_id: Option<LogId<CLID>>,

    last_membership: StoredMembership<CLID, NID, N, M>,

    /// Reserved. Cannot be [`serde::de::IgnoredAny`]: that requires `deserialize_any`, which a
    /// positional format cannot provide.
    #[serde(default)]
    #[allow(dead_code)]
    snapshot_id: crate::SnapshotId,
}

#[cfg(feature = "serde")]
impl<CLID, NID, N, M> From<SnapshotMetaWire<CLID, NID, N, M>> for SnapshotMeta<CLID, NID, N, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    fn from(wire: SnapshotMetaWire<CLID, NID, N, M>) -> Self {
        SnapshotMeta {
            last_log_id: wire.last_log_id,
            last_membership: wire.last_membership,
        }
    }
}

/// Hand-written rather than `#[serde(into = "SnapshotMetaWire<CLID, NID, N>")]`, which would
/// require `Self: Clone` and thus narrow this impl to `N: Clone`.
#[cfg(feature = "serde")]
impl<CLID, NID, N, M> serde::Serialize for SnapshotMeta<CLID, NID, N, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        use serde::ser::SerializeStruct;

        // The field count is the compatibility contract: it must stay 3.
        let mut s = serializer.serialize_struct("SnapshotMeta", 3)?;
        s.serialize_field("last_log_id", &self.last_log_id)?;
        s.serialize_field("last_membership", &self.last_membership)?;
        s.serialize_field("snapshot_id", "")?;
        s.end()
    }
}

impl<CLID, NID, N, M> Default for SnapshotMeta<CLID, NID, N, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    fn default() -> Self {
        Self {
            last_log_id: None,
            last_membership: StoredMembership::default(),
        }
    }
}

impl<CLID, NID, N, M> fmt::Display for SnapshotMeta<CLID, NID, N, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{last_log:{}, last_membership: {}}}",
            self.last_log_id.display(),
            self.last_membership
        )
    }
}

impl<CLID, NID, N, M> SnapshotMeta<CLID, NID, N, M>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    /// Get the signature of this snapshot metadata for comparison and identification.
    pub fn signature(&self) -> SnapshotSignature<CLID> {
        SnapshotSignature {
            last_log_id: self.last_log_id.clone(),
            last_membership_log_id: self.last_membership.log_id().as_ref().map(|x| Box::new(x.clone())),
        }
    }

    /// Returns a ref to the id of the last log that is included in this snapshot.
    pub fn last_log_id(&self) -> Option<&LogId<CLID>> {
        self.last_log_id.as_ref()
    }
}

#[cfg(test)]
mod tests {

    #[cfg(feature = "serde")]
    #[test]
    fn test_snapshot_meta_serde() {
        use maplit::btreeset;

        use crate::Membership;
        use crate::StoredMembership;
        use crate::engine::testing::UTConfig;
        use crate::engine::testing::log_id;
        use crate::type_config::alias::SnapshotMetaOf;

        let meta = SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership: StoredMembership::new(
                Some(log_id(4, 5, 6)),
                Membership::new_with_defaults(vec![btreeset! {1,2}], []),
            ),
        };

        // Three fields, the last an empty `snapshot_id`. The count is the compatibility
        // contract: a positional format (bincode, postcard) encodes only the count, so this
        // assertion is what keeps 0.9 data readable there too.
        let want = r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership":{"log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"membership":{"configs":[[1,2]],"nodes":{"1":null,"2":null}}},"snapshot_id":""}"#;

        assert_eq!(want, serde_json::to_string(&meta).unwrap());
        assert_eq!(meta, serde_json::from_str::<SnapshotMetaOf<UTConfig>>(want).unwrap());

        // Data written by openraft 0.9, when `snapshot_id` was still a field of `SnapshotMeta`,
        // deserializes to the same value; the id is ignored.
        let v09 = r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership":{"log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"membership":{"configs":[[1,2]],"nodes":{"1":null,"2":null}}},"snapshot_id":"1-2-3-4"}"#;

        assert_eq!(meta, serde_json::from_str::<SnapshotMetaOf<UTConfig>>(v09).unwrap());
    }
}
