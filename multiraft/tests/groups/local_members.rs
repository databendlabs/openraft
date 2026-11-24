use std::collections::HashMap;
use std::future::Future;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::OnceLock;

use openraft::OptionalSend;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;
use openraft_multi::GroupRouter;

use crate::C;
use crate::Raft;

/// Delivers each request to the target member of its group in this process.
///
/// This is the router of `benchmarks/idle-groups`, without its count of AppendEntries requests.
#[derive(Clone, Default)]
pub(crate) struct LocalMembers {
    /// Every member, by group id and node id, set once every member exists.
    members: Arc<OnceLock<HashMap<(u64, u64), Raft>>>,
}

impl LocalMembers {
    /// Deliver requests to `groups`, where `groups[g][i]` is node `i + 1` of group `g`.
    pub(crate) fn set_members(&self, groups: &[Vec<Raft>]) {
        let mut members = HashMap::new();
        for (group_id, group) in (0u64..).zip(groups) {
            for (node_id, raft) in (1u64..).zip(group) {
                members.insert((group_id, node_id), raft.clone());
            }
        }

        if self.members.set(members).is_err() {
            panic!("the members are set once");
        }
    }

    fn member(&self, group_id: u64, target: u64) -> Result<&Raft, Unreachable<C>> {
        let member = self.members.get().and_then(|members| members.get(&(group_id, target)));
        member.ok_or_else(|| Unreachable::from_string(format!("group {group_id} has no member {target}")))
    }
}

impl GroupRouter<C, u64> for LocalMembers {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn append_entries(
        &self,
        target: u64,
        group_id: u64,
        rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        let member = self.member(group_id, target)?;
        member.append_entries(rpc).await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))
    }

    async fn vote(
        &self,
        target: u64,
        group_id: u64,
        rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        let member = self.member(group_id, target)?;
        member.vote(rpc).await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))
    }

    async fn pre_vote(
        &self,
        target: u64,
        group_id: u64,
        rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        let member = self.member(group_id, target)?;
        member.pre_vote(rpc).await.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))
    }

    async fn full_snapshot(
        &self,
        target: u64,
        group_id: u64,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        let member = self.member(group_id, target)?;
        member
            .install_full_snapshot(vote, snapshot)
            .await
            .map_err(|e| StreamingError::Unreachable(Unreachable::new(&e)))
    }
}
