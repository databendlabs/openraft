//! Defines the [`NetSnapshot`] trait for snapshot transmission.

use std::future::Future;

use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::network::RPCOption;
use crate::raft::SnapshotResponse;
use crate::type_config::alias::SnapshotOf;
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
    /// Snapshot data this network implementation can transmit.
    #[since(version = "0.10.0", change = "moved SnapshotData from RaftTypeConfig to NetSnapshot")]
    type SnapshotData: OptionalSend + 'static;

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
    /// Return `Ok` only with the response that [`Raft::install_full_snapshot()`] returned on the
    /// target. The leader takes it as proof that the target installed the snapshot or already had
    /// every log it covers: it records `snapshot.meta.last_log_id` as the target's matching log id
    /// and replicates the logs after it. A response whose vote is greater than `vote` is a
    /// rejection instead: the leader records nothing for it and steps down.
    ///
    /// Do not return `Ok` for a snapshot that the target has not installed, such as one it only
    /// staged to install at a later restart, or one whose [`Raft::install_full_snapshot()`]
    /// returned an error. Return an error instead, such as [`StreamingError::Unreachable`] for a
    /// target that is restarting: the leader records nothing for it and sends the snapshot again
    /// after a backoff.
    ///
    /// `cancel` gets `Ready` when the caller decides to cancel this snapshot transmission.
    /// The network implementation is also responsible for enforcing `option.soft_ttl()`.
    ///
    /// [`Raft::install_full_snapshot()`]: crate::raft::Raft::install_full_snapshot
    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>>;
}
