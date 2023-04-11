use async_trait::async_trait;

use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

/// The trait to build a [`RaftStorage`] implementation.
///
/// The generic parameter `C` is type config for a `RaftStorage` implementation,
/// `S` is the type that implements `RaftStorage`,
/// and `G` is a guard type that cleanup resource when being dropped.
///
/// By default `G` is a trivial guard `()`. To test a store that is backed by a folder on disk, `G`
/// could be the dropper of the temp-dir that stores data.
#[async_trait]
pub trait StoreBuilder<C, S, G = ()>: Send + Sync
where
    C: RaftTypeConfig,
    S: RaftStorage<C>,
{
    /// Build a [`RaftStorage`] implementation
    async fn build(&self) -> Result<(G, S), StorageError<C::NodeId>>;
}
