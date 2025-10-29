use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::Vote;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::blank_ent;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Too many inconsistent log should not block replication.
///
/// Reproduce the bug that when append-entries, if there are more than 50 inconsistent logs the
/// conflict is always set to `last_log`, which blocks replication for ever.
/// https://github.com/drmingdrmer/openraft/blob/79a39970855d80e1d3b761fadbce140ecf1da59e/async-raft/src/core/append_entries.rs#L131-L154
///
/// - fake a cluster of node 0,1,2. R0 has ~100 uncommitted log at term 2. R2 has ~100 uncommitted
///   log at term 3.
///
/// ```
/// R0 ... 2,99 2,100
/// R1
/// R2 ... 3,99 3,100
/// ```
///
/// - Start the cluster and node 2 start to replicate logs.
/// - test the log should be replicated to node 0.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn append_inconsistent_log() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- remove all nodes and fake the logs");

    let (r0, mut sto0, sm0) = router.remove_node(0).unwrap();
    let (r1, sto1, sm1) = router.remove_node(1).unwrap();
    let (r2, mut sto2, sm2) = router.remove_node(2).unwrap();

    r0.shutdown().await?;
    r1.shutdown().await?;
    r2.shutdown().await?;

    for i in log_index + 1..=100 {
        sto0.blocking_append([blank_ent(2, 1, i)]).await?;
        sto2.blocking_append([blank_ent(3, 3, i)]).await?;
    }

    sto0.save_vote(&Vote::new(4, 1)).await?;
    sto2.save_vote(&Vote::new(3, 3)).await?;

    log_index = 100;

    tracing::info!(
        log_index,
        "--- restart node 1 and isolate. To let node-2 to become leader, node-1 should not vote for node-0"
    );
    {
        router.new_raft_node_with_sto(1, sto1.clone(), sm1.clone()).await;
        router.set_network_error(1, true);
    }

    tracing::info!(log_index, "--- restart node 0 and 2");
    {
        router.new_raft_node_with_sto(0, sto0.clone(), sm0.clone()).await;
        router.new_raft_node_with_sto(2, sto2.clone(), sm2.clone()).await;
    }

    // leader appends at least one blank log. There may be more than one transient leaders
    log_index += 1;

    tracing::info!(log_index, "--- wait for node states");
    {
        router
            .wait(&0, Some(Duration::from_millis(2000)))
            .state(ServerState::Follower, "node 0 become follower")
            .await?;

        router
            .wait(&2, Some(Duration::from_millis(5000)))
            .state(ServerState::Leader, "node 2 become leader")
            .await?;
    }

    router
        .wait(&0, Some(Duration::from_millis(2000)))
        // leader appends at least one blank log. There may be more than one transient leaders
        .applied_index_at_least(Some(log_index), "sync log to node 0")
        .await?;

    let logs = sto0.try_get_log_entries(60..=60).await?;
    assert_eq!(
        3,
        logs.first().unwrap().log_id.committed_leader_id().term,
        "log is overridden by leader logs"
    );

    Ok(())
}
