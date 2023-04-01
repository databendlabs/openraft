use async_trait::async_trait;

use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::RaftTypeConfig;
use crate::StorageError;

/// The trait to build a [`RaftLogStorage`] and [`RaftStateMachine`] implementation.
///
/// The generic parameter `C` is type config for a `RaftLogStorage` and `RaftStateMachine`
/// implementation,
/// `LS` is the type that implements `RaftLogStorage`,
/// `SM` is the type that implements `RaftStateMachine`,
/// and `G` is a guard type that cleanup resource when being dropped.
///
/// By default `G` is a trivial guard `()`. To test a store that is backed by a folder on disk, `G`
/// could be the dropper of the temp-dir that stores data.
#[async_trait]
pub trait StoreBuilder<C, LS, SM, G = ()>: Send + Sync
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// Build a [`RaftLogStorage`] implementation
    async fn build(&self) -> Result<(G, LS, SM), StorageError<C::NodeId>>;
}
