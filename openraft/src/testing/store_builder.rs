use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;

use async_trait::async_trait;

use crate::AppData;
use crate::AppDataResponse;
use crate::DefensiveCheckBase;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoreExt;

/// The trait to build a [`RaftStorage`] implementation.
#[async_trait]
pub trait StoreBuilder<C, S>: Send + Sync
where
    C: RaftTypeConfig,
    S: RaftStorage<C>,
{
    async fn run_test<Fun, Ret, Res>(&self, t: Fun) -> Result<Ret, StorageError<C::NodeId>>
    where
        Res: Future<Output = Result<Ret, StorageError<C::NodeId>>> + Send,
        Fun: Fn(S) -> Res + Sync + Send;
}

/// A builder for testing [`StoreExt`].
pub struct DefensiveStoreBuilder<C, BaseStore, BaseBuilder>
where
    C: RaftTypeConfig,
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
    C: RaftTypeConfig,
    C::D: AppData + Debug,
    C::R: AppDataResponse + Debug,
    BaseStore: RaftStorage<C>,
    BaseBuilder: StoreBuilder<C, BaseStore>,
{
    async fn run_test<Fun, Ret, Res>(&self, t: Fun) -> Result<Ret, StorageError<C::NodeId>>
    where
        Res: Future<Output = Result<Ret, StorageError<C::NodeId>>> + Send,
        Fun: Fn(StoreExt<C, BaseStore>) -> Res + Sync + Send,
    {
        self.base_builder
            .run_test(|base_store| async {
                let sto_ext = StoreExt::new(base_store);
                sto_ext.set_defensive(true);
                assert!(sto_ext.is_defensive(), "must impl defensive check");
                t(sto_ext).await
            })
            .await
    }
}
