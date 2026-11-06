use std::fmt;
use std::future::Future;
use std::sync::Weak;

use openraft::OptionalSend;
use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::async_runtime::Select2;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::RaftStateMachine;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;

use crate::Registry;

/// Sends the RPCs of node `source` to node `target` by calling the target's [`Raft`] handle.
///
/// An RPC fails with [`Unreachable`] when the registry is dropped, when the target is not
/// registered, or when the target returns `Fatal` because it has stopped.
///
/// A call between two tasks of one process cannot time out, so the connection ignores the
/// [`RPCOption`] of every RPC.
pub struct Connection<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    source: C::NodeId,
    target: C::NodeId,
    registry: Weak<Registry<C, SM>>,
}

impl<C, SM> Connection<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(source: C::NodeId, target: C::NodeId, registry: Weak<Registry<C, SM>>) -> Self {
        Self {
            source,
            target,
            registry,
        }
    }

    /// Returns a clone of the target's handle.
    fn target_raft(&self) -> Result<Raft<C, SM>, Unreachable<C>> {
        let Some(registry) = self.registry.upgrade() else {
            return Err(self.unreachable("the registry is dropped"));
        };

        let Some(raft) = registry.get(&self.target) else {
            return Err(self.unreachable("the target is not registered"));
        };

        Ok(raft)
    }

    fn unreachable(&self, reason: impl fmt::Display) -> Unreachable<C> {
        Unreachable::from_string(format!("{} -> {}: {}", self.source, self.target, reason))
    }
}

impl<C, SM> RaftNetworkV2<C> for Connection<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    type SnapshotData = SM::SnapshotData;

    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        let target = self.target_raft()?;
        let res = target.append_entries(rpc).await;
        let resp = res.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }

    async fn vote(&mut self, rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let target = self.target_raft()?;
        let res = target.vote(rpc).await;
        let resp = res.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }

    async fn pre_vote(&mut self, rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let target = self.target_raft()?;
        let res = target.pre_vote(rpc).await;
        let resp = res.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        let target = self.target_raft()?;
        let install = target.install_full_snapshot(vote, snapshot);

        match C::select2(cancel, install).await {
            Select2::First(closed) => Err(StreamingError::Closed(closed)),
            Select2::Second(res) => {
                let resp = res.map_err(|e| self.unreachable(e))?;
                Ok(resp)
            }
        }
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<C>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<C>, RPCError<C>> {
        let target = self.target_raft()?;
        let res = target.handle_transfer_leader(req).await;
        let resp = res.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }
}
