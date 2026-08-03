use openraft_macros::since;

use crate::log_id::LogId;
use crate::vote::RaftCommittedLeaderId;

/// A small piece of information for identifying a snapshot and error tracing.
///
/// A snapshot is identified by the position it covers: two snapshots at the same `last_log_id`
/// cover the same state, even when they differ in bytes. The 0.9 `snapshot_id` identified a
/// transfer, not the snapshot, and is gone from the API.
///
/// # Compatibility with 0.9
///
/// Before 0.10.0 this type also carried the `snapshot_id`, declared last. Signatures are not
/// stored, but they travel inside errors: a `Fatal(StorageError)` returned over the v1 protocol
/// carries one to a 0.9 peer. Under a positional format (`bincode`, `postcard`,
/// `rmp-serde::to_vec`) a changed field count makes such a record unreadable, and nested inside
/// the error enums it can be misparsed silently instead of failing.
///
/// So, as in [`SnapshotMeta`](crate::storage::SnapshotMeta), the slot is reserved rather than
/// removed: this type serializes as three fields, the third being an always-empty
/// `snapshot_id`, and ignores that field when reading. 0.9 and 0.10 signatures are therefore
/// interchangeable in both directions, under named and positional formats alike.
///
/// The guarantee covers this type alone. [`StorageError`](crate::StorageError), which carries
/// the signature, was an enum in 0.9 and is a struct in 0.10, so a whole 0.10 error is not
/// parseable by a 0.9 peer regardless. Reserving the slot means the signature is never the
/// incompatibility; peers of a different version should treat error bodies as diagnostic
/// rather than parse them.
#[since(version = "0.10.0", change = "removed `snapshot_id`")]
#[since(
    version = "0.10.0",
    change = "from `SnapshotSignature<C>` to `SnapshotSignature<CLID>`"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize),
    serde(bound = "", from = "SnapshotSignatureWire<CLID>")
)]
pub struct SnapshotSignature<CLID>
where CLID: RaftCommittedLeaderId
{
    /// Log entries up to which this snapshot includes, inclusive.
    pub last_log_id: Option<LogId<CLID>>,

    /// The last applied membership log id.
    pub last_membership_log_id: Option<Box<LogId<CLID>>>,
}

/// The serialized layout of [`SnapshotSignature`], which still carries the 0.9 `snapshot_id`
/// slot.
///
/// Written always-empty and ignored on read; see the compatibility note on
/// [`SnapshotSignature`].
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(bound = "")]
struct SnapshotSignatureWire<CLID>
where CLID: RaftCommittedLeaderId
{
    last_log_id: Option<LogId<CLID>>,

    last_membership_log_id: Option<Box<LogId<CLID>>>,

    /// Reserved. Cannot be [`serde::de::IgnoredAny`]: that requires `deserialize_any`, which a
    /// positional format cannot provide.
    #[serde(default)]
    #[allow(dead_code)]
    snapshot_id: crate::SnapshotId,
}

#[cfg(feature = "serde")]
impl<CLID> From<SnapshotSignatureWire<CLID>> for SnapshotSignature<CLID>
where CLID: RaftCommittedLeaderId
{
    fn from(wire: SnapshotSignatureWire<CLID>) -> Self {
        SnapshotSignature {
            last_log_id: wire.last_log_id,
            last_membership_log_id: wire.last_membership_log_id,
        }
    }
}

/// Hand-written rather than `#[serde(into = "SnapshotSignatureWire<CLID>")]`, which would
/// require `Self: Clone` and thus narrow this impl to `CLID: Clone`.
#[cfg(feature = "serde")]
impl<CLID> serde::Serialize for SnapshotSignature<CLID>
where CLID: RaftCommittedLeaderId
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        use serde::ser::SerializeStruct;

        // The field count is the compatibility contract: it must stay 3.
        let mut s = serializer.serialize_struct("SnapshotSignature", 3)?;
        s.serialize_field("last_log_id", &self.last_log_id)?;
        s.serialize_field("last_membership_log_id", &self.last_membership_log_id)?;
        s.serialize_field("snapshot_id", "")?;
        s.end()
    }
}

#[cfg(test)]
mod tests {

    #[cfg(feature = "serde")]
    #[test]
    fn test_snapshot_signature_serde() {
        use super::SnapshotSignature;
        use crate::engine::testing::UtClid;
        use crate::engine::testing::log_id;

        let sig = SnapshotSignature::<UtClid> {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership_log_id: Some(Box::new(log_id(4, 5, 6))),
        };

        // Three fields, the last an empty `snapshot_id`. The count is the compatibility
        // contract: a positional format (bincode, postcard) encodes only the count, so this
        // assertion is what keeps errors readable to 0.9 peers there too.
        let want = r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership_log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"snapshot_id":""}"#;

        assert_eq!(want, serde_json::to_string(&sig).unwrap());
        assert_eq!(sig, serde_json::from_str::<SnapshotSignature<UtClid>>(want).unwrap());

        // Data written by openraft 0.9, when `snapshot_id` was still a field, deserializes to
        // the same value; the id is ignored.
        let v09 = r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership_log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"snapshot_id":"1-2-3-4"}"#;

        assert_eq!(sig, serde_json::from_str::<SnapshotSignature<UtClid>>(v09).unwrap());
    }
}
