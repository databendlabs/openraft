use openraft_macros::since;

use crate::SnapshotId;
use crate::log_id::LogId;
use crate::vote::RaftCommittedLeaderId;

/// A small piece of information for identifying a snapshot and error tracing.
///
/// # Compatibility with 0.9
///
/// In 0.9 `snapshot_id` was a plain [`SnapshotId`], because every snapshot carried one. It is now
/// an `Option`, since a snapshot that is not being transferred has no id.
///
/// The serialized form is unchanged: a plain string, with the empty string standing for `None`.
/// Making the field itself an `Option` on the wire would have broken 0.9 data under a positional
/// format (`bincode`, `postcard`, `rmp-serde::to_vec`), where an `Option` is encoded with a leading
/// discriminant that a plain string does not have.
#[since(version = "0.10.0", change = "`snapshot_id` became an `Option`")]
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

    /// To identify a snapshot when transferring.
    ///
    /// This is a transfer-time value: it is `None` for a snapshot that is not being transferred,
    /// such as one built locally or restored from storage on startup. It is serialized as the
    /// empty string.
    pub snapshot_id: Option<SnapshotId>,
}

impl<CLID> SnapshotSignature<CLID>
where CLID: RaftCommittedLeaderId
{
    /// Attach the id of the transfer this snapshot is part of.
    pub fn with_snapshot_id(mut self, snapshot_id: SnapshotId) -> Self {
        self.snapshot_id = Some(snapshot_id);
        self
    }
}

/// The serialized layout of [`SnapshotSignature`], which keeps the 0.9 non-optional `snapshot_id`.
///
/// See the compatibility note on [`SnapshotSignature`].
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(bound = "")]
struct SnapshotSignatureWire<CLID>
where CLID: RaftCommittedLeaderId
{
    last_log_id: Option<LogId<CLID>>,

    last_membership_log_id: Option<Box<LogId<CLID>>>,

    #[serde(default)]
    snapshot_id: SnapshotId,
}

#[cfg(feature = "serde")]
impl<CLID> From<SnapshotSignatureWire<CLID>> for SnapshotSignature<CLID>
where CLID: RaftCommittedLeaderId
{
    fn from(wire: SnapshotSignatureWire<CLID>) -> Self {
        SnapshotSignature {
            last_log_id: wire.last_log_id,
            last_membership_log_id: wire.last_membership_log_id,
            snapshot_id: Some(wire.snapshot_id).filter(|id| !id.is_empty()),
        }
    }
}

/// Hand-written rather than `#[serde(into = "SnapshotSignatureWire<CLID>")]`, which would require
/// `Self: Clone` and thus narrow this impl to `CLID: Clone`.
#[cfg(feature = "serde")]
impl<CLID> serde::Serialize for SnapshotSignature<CLID>
where CLID: RaftCommittedLeaderId
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        use serde::ser::SerializeStruct;

        let mut s = serializer.serialize_struct("SnapshotSignature", 3)?;
        s.serialize_field("last_log_id", &self.last_log_id)?;
        s.serialize_field("last_membership_log_id", &self.last_membership_log_id)?;
        s.serialize_field("snapshot_id", self.snapshot_id.as_deref().unwrap_or(""))?;
        s.end()
    }
}

#[cfg(test)]
mod tests {

    #[cfg(feature = "serde")]
    #[test]
    fn test_snapshot_signature_serde() {
        use super::SnapshotSignature;
        use crate::engine::testing::log_id;

        let sig = SnapshotSignature {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership_log_id: Some(Box::new(log_id(4, 5, 6))),
            snapshot_id: Some("test".to_string()),
        };
        let s = serde_json::to_string(&sig).unwrap();
        assert_eq!(
            s,
            r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership_log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"snapshot_id":"test"}"#
        );
        let sig2: SnapshotSignature<crate::engine::testing::UtClid> = serde_json::from_str(&s).unwrap();
        assert_eq!(sig, sig2);
    }

    /// `snapshot_id` is an `Option` in the API but a plain string on the wire, so that data
    /// written by 0.9 stays readable under positional formats too. `None` is the empty string.
    #[cfg(feature = "serde")]
    #[test]
    fn test_snapshot_signature_serde_no_snapshot_id() {
        use super::SnapshotSignature;
        use crate::engine::testing::UtClid;
        use crate::engine::testing::log_id;

        let sig = SnapshotSignature::<UtClid> {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership_log_id: None,
            snapshot_id: None,
        };

        // Three fields, the id an empty string: the same shape 0.9 wrote.
        let want = r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership_log_id":null,"snapshot_id":""}"#;

        assert_eq!(want, serde_json::to_string(&sig).unwrap());
        assert_eq!(sig, serde_json::from_str::<SnapshotSignature<UtClid>>(want).unwrap());
    }
}
