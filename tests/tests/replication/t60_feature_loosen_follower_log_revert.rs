use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// With "allow_log_reversion", the leader allows follower to revert its log to an
/// earlier state.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn feature_loosen_follower_log_revert() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            // Make sure the replication is done in more than one steps
            max_payload_entries: 1,
            allow_log_reversion: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;

    tracing::info!(log_index, "--- write 10 logs");
    {
        log_index += router.client_request_many(0, "0", 10).await?;
        for i in [0, 1, 2, 3] {
            router.wait(&i, timeout()).applied_index(Some(log_index), format!("{} writes", 10)).await?;
        }
    }

    tracing::info!(log_index, "--- erase node 3 and restart");
    {
        let (_raft, _ls, _sm) = router.remove_node(3).unwrap();
        let (log, sm) = openraft_memstore::new_mem_store();

        router.new_raft_node_with_sto(3, log, sm).await;
        router.add_learner(0, 3).await?;
        log_index += 1; // add learner
    }

    tracing::info!(log_index, "--- write another 10 logs, leader should not panic");
    {
        log_index += router.client_request_many(0, "0", 10).await?;
        for i in [0, 1, 2, 3] {
            router.wait(&i, timeout()).applied_index(Some(log_index), format!("{} writes", 10)).await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
