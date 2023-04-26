use std::path::Path;
use std::sync::Arc;

use openraft::compat;

/// A builder builds openraft-0.7 based rocksstore.
struct Builder07;
/// A builder builds latest openraft based rocksstore.
struct BuilderLatest;

#[async_trait::async_trait]
impl compat::testing::StoreBuilder07 for Builder07 {
    type D = rocksstore07::RocksRequest;
    type R = rocksstore07::RocksResponse;
    type S = Arc<rocksstore07::RocksStore>;

    async fn build(&self, p: &Path) -> Arc<rocksstore07::RocksStore> {
        rocksstore07::RocksStore::new(p).await
    }

    fn sample_app_data(&self) -> Self::D {
        rocksstore07::RocksRequest::Set {
            key: s("foo"),
            value: s("bar"),
        }
    }
}

#[async_trait::async_trait]
impl compat::testing::StoreBuilder for BuilderLatest {
    type C = crate::TypeConfig;
    type S = Arc<crate::RocksStore>;

    async fn build(&self, p: &Path) -> Arc<crate::RocksStore> {
        crate::RocksStore::new(p).await
    }

    fn sample_app_data(&self) -> <<Self as compat::testing::StoreBuilder>::C as openraft::RaftTypeConfig>::D {
        crate::RocksRequest::Set {
            key: s("foo"),
            value: s("bar"),
        }
    }
}

#[tokio::test]
async fn test_compatibility_with_rocksstore_07() -> anyhow::Result<()> {
    let suite = compat::testing::Suite07 {
        builder07: Builder07,
        builder_latest: BuilderLatest,
    };

    suite.test_all().await?;

    Ok(())
}

fn s(v: impl ToString) -> String {
    v.to_string()
}
