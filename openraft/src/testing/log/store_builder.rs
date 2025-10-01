use openraft_macros::add_async_trait;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;

/// The trait to build a [`RaftLogStorage`] and [`RaftStateMachine`] implementation.
///
/// The generic parameter `C` is type config for a `RaftLogStorage` and `RaftStateMachine`
/// implementation,
/// `LS` is the type that implements `RaftLogStorage`,
/// `SM` is the type that implements `RaftStateMachine`,
/// and `G` is a guard type that cleans up resources when being dropped.
///
/// By default, `G` is a trivial guard `()`. To test a store that is backed by a folder on disk, `G`
/// could be the dropper of the temp-dir that stores data.
#[add_async_trait]
pub trait StoreBuilder<C, LS, SM, G = ()>: Send + Sync
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// Build a [`RaftLogStorage`] and [`RaftStateMachine`] implementation
    async fn build(&self) -> Result<(G, LS, SM), StorageError<C>>;
}
