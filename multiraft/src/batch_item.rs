use openraft::RaftTypeConfig;
use openraft::openraft_macros::since;
use openraft::raft::AppendEntriesRequest;

/// One AppendEntries request of a batch, addressed to one Raft group on the target node.
///
/// A batch carries complete requests: each group's vote, `prev_log_id` and `leader_commit` stay as
/// that group sent them. The receiver passes `rpc` to `Raft::append_entries()` of `group_id`, and
/// replies with a [`BatchReply`](crate::BatchReply) that carries the same `id`.
#[since(version = "0.10.1")]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(serialize = "G: serde::Serialize", deserialize = "G: serde::Deserialize<'de>"))
)]
pub struct BatchItem<C, G>
where C: RaftTypeConfig
{
    /// Identifies this item among the items of its batch; its reply carries it back.
    pub id: u64,

    /// The Raft group on the target node that receives `rpc`.
    pub group_id: G,

    /// The complete request for that group.
    pub rpc: AppendEntriesRequest<C>,
}
