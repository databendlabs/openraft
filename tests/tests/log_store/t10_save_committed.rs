use anyhow::Result;
use openraft::Config;
use openraft::storage::RaftLogStorage;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Before applying log, write `committed` log id to log store.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_committed_log_id_to_log_store() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    let written = router.client_request_many(0, "0", 10).await?;
    log_index += written.count;

    for i in [0, 1, 2] {
        router.wait(&i).applied_index(Some(log_index), "write logs").await?;
    }

    for id in [0, 1, 2] {
        let Node { log_store: mut ls, .. } = router.stop_node(id).await?;
        let committed = ls.read_committed().await?;
        assert_eq!(Some(log_id(1, 0, log_index)), committed, "node-{} committed", id);
    }

    Ok(())
}
