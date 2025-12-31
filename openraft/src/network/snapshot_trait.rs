//! Defines the [`NetSnapshot`] trait for snapshot transmission.

use std::future::Future;

use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::RPCOption;
use crate::raft::SnapshotResponse;
use crate::storage::Snapshot;
use crate::type_config::alias::VoteOf;

/// Sends full snapshots to a target node.
///
/// **For most applications, implement [`RaftNetworkV2`] instead.** This trait is
/// automatically derived from `RaftNetworkV2` via blanket implementation.
///
/// Direct implementation is an advanced option for fine-grained control.
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
#[add_async_trait]
pub trait NetSnapshot<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send a complete Snapshot to the target.
    ///
    /// This method is responsible for fragmenting the snapshot and sending it to the target node.
    /// Before returning from this method, the snapshot should be completely transmitted and
    /// installed on the target node or rejected because of `vote` being smaller than the
    /// remote one.
    ///
    /// The `vote` is the leader vote used to check if the leader is still valid by a
    /// follower.
    /// When the follower finished receiving the snapshot, it calls
    /// [`Raft::install_full_snapshot()`] with this vote.
    ///
    /// `cancel` gets `Ready` when the caller decides to cancel this snapshot transmission.
    ///
    /// [`Raft::install_full_snapshot()`]: crate::raft::Raft::install_full_snapshot
    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>>;
}
