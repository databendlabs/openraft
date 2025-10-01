use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::storage::RaftLogStorage;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Before applying log, write `committed` log id to log store.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_committed_log_id_to_log_store() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    log_index += router.client_request_many(0, "0", 10).await?;

    for i in [0, 1, 2] {
        router.wait(&i, timeout()).applied_index(Some(log_index), "write logs").await?;
    }

    for id in [0, 1, 2] {
        let (_, mut ls, _) = router.remove_node(id).unwrap();
        let committed = ls.read_committed().await?;
        assert_eq!(Some(log_id(1, 0, log_index)), committed, "node-{} committed", id);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
