use std::marker::PhantomData;

use async_trait::async_trait;

use crate::DefensiveCheckBase;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoreExt;

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

/// A builder for testing [`StoreExt`].
pub struct DefensiveStoreBuilder<C, BaseStore, BaseBuilder, G>
where
    C: RaftTypeConfig,
    BaseStore: RaftStorage<C>,
    BaseBuilder: StoreBuilder<C, BaseStore, G>,
{
    pub base_builder: BaseBuilder,

    pub s: PhantomData<(C, BaseStore, G)>,
}

#[async_trait]
impl<C, G, BaseStore, BaseBuilder> StoreBuilder<C, StoreExt<C, BaseStore>, G>
    for DefensiveStoreBuilder<C, BaseStore, BaseBuilder, G>
where
    C: RaftTypeConfig,
    BaseStore: RaftStorage<C>,
    BaseBuilder: StoreBuilder<C, BaseStore, G>,
    G: Send + Sync,
{
    async fn build(&self) -> Result<(G, StoreExt<C, BaseStore>), StorageError<C::NodeId>> {
        let (g, store) = self.base_builder.build().await?;
        let sto_ext = StoreExt::new(store);
        sto_ext.set_defensive(true);
        assert!(sto_ext.is_defensive(), "must impl defensive check");

        Ok((g, sto_ext))
    }
}
