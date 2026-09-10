use log_rocks::RocksLogStore;
use openraft::StorageError;
use openraft::storage::RaftLogStorage;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::LeaderIdOf;
use openraft::vote::RaftLeaderId;
use tempfile::TempDir;

use crate::RocksStateMachine;
use crate::TypeConfig;

struct RocksBuilder {}

impl StoreBuilder<TypeConfig, RocksLogStore<TypeConfig>, RocksStateMachine, TempDir> for RocksBuilder {
    async fn build(&self) -> Result<(TempDir, RocksLogStore<TypeConfig>, RocksStateMachine), StorageError<TypeConfig>> {
        let td = TempDir::new().map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        let (log_store, sm) =
            crate::new(td.path()).await.map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        Ok((td, log_store, sm))
    }
}

#[test]
pub fn test_rocks_store() {
    TypeConfig::run(async {
        Suite::test_all(RocksBuilder {}).await.unwrap();
    });
}

#[test]
fn test_local_retirement_survives_reopen() {
    TypeConfig::run(async {
        let temp_dir = TempDir::new().unwrap();
        let retired_for = LeaderIdOf::<TypeConfig>::new(3, 1);

        {
            let (mut store, state_machine) = crate::new::<TypeConfig, _>(temp_dir.path()).await.unwrap();
            assert_eq!(None, store.read_local_retirement().await.unwrap());
            store.save_local_retirement(&retired_for).await.unwrap();
            drop(state_machine);
        }

        let (mut reopened, _state_machine) = crate::new::<TypeConfig, _>(temp_dir.path()).await.unwrap();
        assert_eq!(Some(retired_for), reopened.read_local_retirement().await.unwrap());
    });
}
