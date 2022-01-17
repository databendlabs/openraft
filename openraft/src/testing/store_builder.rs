use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;

use async_trait::async_trait;

use crate::AppData;
use crate::AppDataResponse;
use crate::DefensiveCheck;
use crate::RaftStorage;
use crate::StoreExt;

/// The trait to build a [`RaftStorage`] implementation.
#[async_trait]
pub trait StoreBuilder<D, R, S>: Send + Sync
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
{
    async fn build(&self) -> S;
}

/// Make the tests easy to use by converting a closure to a [`StoreBuilder`].
///
/// E.g. to run tests on your [`RaftStorage`] implementation, just use `Suite::test_all(|| new_store())`,
/// if your have already provided `async fn new_store() -> MyStore`
#[async_trait]
impl<D, R, S, Fu, F> StoreBuilder<D, R, S> for F
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
    Fu: Future<Output = S> + Send,
    F: Fn() -> Fu + Sync + Send,
{
    async fn build(&self) -> S {
        (self)().await
    }
}

/// A builder for testing [`StoreExt`].
pub struct DefensiveStoreBuilder<D, R, BaseStore, BaseBuilder>
where
    D: AppData + Debug,
    R: AppDataResponse + Debug,
    BaseStore: RaftStorage<D, R>,
    BaseBuilder: StoreBuilder<D, R, BaseStore>,
{
    pub base_builder: BaseBuilder,

    pub d: PhantomData<D>,
    pub r: PhantomData<R>,
    pub s: PhantomData<BaseStore>,
}

#[async_trait]
impl<D, R, BaseStore, BaseBuilder> StoreBuilder<D, R, StoreExt<D, R, BaseStore>>
    for DefensiveStoreBuilder<D, R, BaseStore, BaseBuilder>
where
    D: AppData + Debug,
    R: AppDataResponse + Debug,
    BaseStore: RaftStorage<D, R>,
    BaseBuilder: StoreBuilder<D, R, BaseStore>,
{
    async fn build(&self) -> StoreExt<D, R, BaseStore> {
        let sto = self.base_builder.build().await;
        let sto_ext = StoreExt::new(sto);
        sto_ext.set_defensive(true);

        assert!(sto_ext.is_defensive(), "must impl defensive check");
        sto_ext
    }
}
