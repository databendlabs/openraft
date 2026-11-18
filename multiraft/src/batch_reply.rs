use openraft::RaftTypeConfig;
use openraft::error::RPCError;
use openraft::openraft_macros::since;
use openraft::raft::AppendEntriesResponse;

/// The reply to one [`BatchItem`](crate::BatchItem).
///
/// The sender matches a reply to its item by `id` alone, so replies may arrive in any order.
#[since(version = "0.10.1")]
#[derive(Clone)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(serialize = "G: serde::Serialize", deserialize = "G: serde::Deserialize<'de>"))
)]
pub struct BatchReply<C, G>
where C: RaftTypeConfig
{
    /// The `id` of the item this reply answers.
    pub id: u64,

    /// The Raft group the item was addressed to.
    pub group_id: G,

    /// The group's response, or why the item did not reach the group, such as an unknown group.
    pub result: Result<AppendEntriesResponse<C>, RPCError<C>>,
}
