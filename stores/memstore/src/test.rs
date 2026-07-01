use std::sync::Arc;

use openraft::StorageError;
use openraft::storage::RaftLogReader;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;

use crate::MemLogStore;
use crate::MemStateMachine;
use crate::TypeConfig;

struct MemStoreBuilder {}

impl StoreBuilder<TypeConfig, Arc<MemLogStore>, Arc<MemStateMachine>, ()> for MemStoreBuilder {
    async fn build(&self) -> Result<((), Arc<MemLogStore>, Arc<MemStateMachine>), StorageError<TypeConfig>> {
        let (log_store, sm) = crate::new_mem_store();
        Ok(((), log_store, sm))
    }
}

#[test]
pub fn test_mem_store() {
    TypeConfig::run(async {
        Suite::test_all(MemStoreBuilder {}).await.unwrap();
    });
}

/// `MemLogStore` honors the `max_bytes` argument of `limited_get_log_entries`: it stops
/// accumulating once the byte budget would be exceeded, but always returns at least one entry.
#[test]
pub fn test_limited_get_log_entries_max_bytes() {
    TypeConfig::run(async {
        let (mut log_store, _sm) = crate::new_mem_store();

        // Feeds blank entries at indices 0..=10.
        Suite::<TypeConfig, Arc<MemLogStore>, Arc<MemStateMachine>, MemStoreBuilder, ()>::feed_10_logs_vote_self(
            &mut log_store,
        )
        .await
        .unwrap();

        // No byte limit: the whole range [1, 11) is returned.
        let all = log_store.limited_get_log_entries(1, 11, None).await.unwrap();
        assert_eq!(10, all.len());

        // A budget below a single entry still returns exactly one entry (progress guarantee).
        let one = log_store.limited_get_log_entries(1, 11, Some(1)).await.unwrap();
        assert_eq!(1, one.len());

        // A budget that exactly fits the first two entries returns exactly two.
        let two_bytes =
            (serde_json::to_string(&all[0]).unwrap().len() + serde_json::to_string(&all[1]).unwrap().len()) as u64;
        let two = log_store.limited_get_log_entries(1, 11, Some(two_bytes)).await.unwrap();
        assert_eq!(2, two.len());

        // One byte short of the second entry truncates back to one.
        let one_again = log_store.limited_get_log_entries(1, 11, Some(two_bytes - 1)).await.unwrap();
        assert_eq!(1, one_again.len());

        // A huge budget behaves like no limit.
        let huge = log_store.limited_get_log_entries(1, 11, Some(u64::MAX)).await.unwrap();
        assert_eq!(10, huge.len());
    });
}
