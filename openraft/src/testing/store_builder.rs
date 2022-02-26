use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;

use async_trait::async_trait;

use crate::AppData;
use crate::AppDataResponse;
use crate::DefensiveCheckBase;
use crate::RaftConfig;
use crate::RaftStorage;
use crate::StoreExt;

/// The trait to build a [`RaftStorage`] implementation.
#[async_trait]
pub trait StoreBuilder<C, S>: Send + Sync
where
    C: RaftConfig,
    S: RaftStorage<C>,
{
    async fn build(&self) -> S;
}

/// Make the tests easy to use by converting a closure to a [`StoreBuilder`].
///
/// E.g. to run tests on your [`RaftStorage`] implementation, just use `Suite::test_all(|| new_store())`,
/// if your have already provided `async fn new_store() -> MyStore`
#[async_trait]
impl<C, S, Fu, F> StoreBuilder<C, S> for F
where
    C: RaftConfig,
    S: RaftStorage<C>,
    Fu: Future<Output = S> + Send,
    F: Fn() -> Fu + Sync + Send,
{
    async fn build(&self) -> S {
        (self)().await
    }
}

/// A builder for testing [`StoreExt`].
pub struct DefensiveStoreBuilder<C, BaseStore, BaseBuilder>
where
    C: RaftConfig,
    C::D: AppData + Debug,
    C::R: AppDataResponse + Debug,
    BaseStore: RaftStorage<C>,
    BaseBuilder: StoreBuilder<C, BaseStore>,
{
    pub base_builder: BaseBuilder,

    pub s: PhantomData<(C, BaseStore)>,
}

#[async_trait]
impl<C, BaseStore, BaseBuilder> StoreBuilder<C, StoreExt<C, BaseStore>>
    for DefensiveStoreBuilder<C, BaseStore, BaseBuilder>
where
    C: RaftConfig,
    C::D: AppData + Debug,
    C::R: AppDataResponse + Debug,
    BaseStore: RaftStorage<C>,
    BaseBuilder: StoreBuilder<C, BaseStore>,
{
    async fn build(&self) -> StoreExt<C, BaseStore> {
        let sto = self.base_builder.build().await;
        let sto_ext = StoreExt::new(sto);
        sto_ext.set_defensive(true);

        assert!(sto_ext.is_defensive(), "must impl defensive check");
        sto_ext
    }
}
