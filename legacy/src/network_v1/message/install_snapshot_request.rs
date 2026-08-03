use std::fmt;

use openraft::RaftTypeConfig;
use openraft::type_config::alias::VoteOf;
use openraft_macros::since;

use super::snapshot_meta::SnapshotMeta;

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
///
/// The serialized layout is identical to the 0.9 `openraft::raft::InstallSnapshotRequest`,
/// so 0.9 and 0.10 peers can exchange this RPC under positional formats too; see
/// [`SnapshotMeta`], which keeps the transfer id in its 0.9 position.
#[since(version = "0.10.0", change = "moved from `openraft::raft::InstallSnapshotRequest`")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotRequest<C>
where C: RaftTypeConfig
{
    /// The leader's current vote.
    pub vote: VoteOf<C>,

    /// Metadata of a snapshot: snapshot_id, last_log_id, membership, etc.
    pub meta: SnapshotMeta<C>,

    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,

    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

impl<C> fmt::Display for InstallSnapshotRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InstallSnapshotRequest {{ vote:{}, meta:{}, offset:{}, len:{}, done:{} }}",
            self.vote,
            self.meta,
            self.offset,
            self.data.len(),
            self.done
        )
    }
}

#[cfg(test)]
mod tests {

    #[cfg(feature = "serde")]
    #[test]
    fn test_install_snapshot_request_serde() {
        use std::collections::BTreeSet;

        use openraft::Membership;
        use openraft::StoredMembership;
        use openraft::Vote;
        use openraft::testing::log_id;

        use super::InstallSnapshotRequest;
        use super::SnapshotMeta;
        use crate::testing::TestConfig;

        let req = InstallSnapshotRequest::<TestConfig> {
            vote: Vote::new_committed(2, 1),
            meta: SnapshotMeta {
                last_log_id: Some(log_id::<TestConfig>(1, 2, 3)),
                last_membership: StoredMembership::new(
                    Some(log_id::<TestConfig>(4, 5, 6)),
                    Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []),
                ),
                snapshot_id: "1-2-3-4".to_string(),
            },
            offset: 7,
            data: vec![1, 2, 3],
            done: true,
        };

        // Five fields with the transfer id inside `meta`: the exact 0.9 wire layout. The
        // field count and order are the compatibility contract with 0.9 peers under
        // positional formats (bincode, postcard), which encode a struct as a bare sequence.
        let want = r#"{"vote":{"leader_id":{"term":2,"node_id":1},"committed":true},"meta":{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership":{"log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"membership":{"configs":[[1,2]],"nodes":{"1":{"addr":"localhost"},"2":{"addr":"localhost"}}}},"snapshot_id":"1-2-3-4"},"offset":7,"data":[1,2,3],"done":true}"#;

        assert_eq!(want, serde_json::to_string(&req).unwrap());
        assert_eq!(
            req,
            serde_json::from_str::<InstallSnapshotRequest<TestConfig>>(want).unwrap()
        );
    }
}
