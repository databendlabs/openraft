use openraft::RaftTypeConfig;
use openraft::raft::SnapshotResponse;
use openraft::type_config::alias::VoteOf;
use openraft_macros::since;

/// The response to an `InstallSnapshotRequest`.
#[since(version = "0.10.0", change = "moved from `openraft::raft::InstallSnapshotResponse`")]
#[derive(Debug)]
#[derive(PartialEq, Eq)]
#[derive(derive_more::Display)]
#[display("{{vote:{}}}", vote)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotResponse<C>
where C: RaftTypeConfig
{
    /// The responder's current vote.
    pub vote: VoteOf<C>,
}

impl<C> From<SnapshotResponse<C>> for InstallSnapshotResponse<C>
where C: RaftTypeConfig
{
    fn from(snap_resp: SnapshotResponse<C>) -> Self {
        Self { vote: snap_resp.vote }
    }
}
