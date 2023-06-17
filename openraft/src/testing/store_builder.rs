#[cfg(not(feature = "storage-v2"))] use std::future::Future;

use async_trait::async_trait;

#[cfg(not(feature = "storage-v2"))] use crate::storage::Adaptor;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
#[cfg(not(feature = "storage-v2"))] use crate::RaftStorage;
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
    /// Build a [`RaftLogStorage`] and [`RaftStateMachine`] implementation
    async fn build(&self) -> Result<(G, LS, SM), StorageError<C::NodeId>>;
}

// Add a default implementation for async function that returns a [`RaftStorage`] implementation.
#[cfg(not(feature = "storage-v2"))]
#[async_trait]
impl<C, S, F, Fu> StoreBuilder<C, Adaptor<C, S>, Adaptor<C, S>, ()> for F
where
    F: Send + Sync,
    C: RaftTypeConfig,
    S: RaftStorage<C>,
    Fu: Future<Output = S> + Send,
    F: Fn() -> Fu,
{
    async fn build(&self) -> Result<((), Adaptor<C, S>, Adaptor<C, S>), StorageError<C::NodeId>> {
        let store = self().await;
        let (log_store, sm) = Adaptor::new(store);
        Ok(((), log_store, sm))
    }
}
