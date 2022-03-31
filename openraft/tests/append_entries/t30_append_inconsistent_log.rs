use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::RaftLogReader;
use openraft::RaftStorage;
use openraft::State;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Too many inconsistent log should not block replication.
///
/// Reproduce the bug that when append-entries, if there are more than 50 inconsistent logs the conflict is always set
/// to `last_log`, which blocks replication for ever.
/// https://github.com/drmingdrmer/openraft/blob/79a39970855d80e1d3b761fadbce140ecf1da59e/async-raft/src/core/append_entries.rs#L131-L154
///
/// - fake a cluster of node 0,1,2. R0 has ~100 uncommitted log at term 2. R2 has ~100 uncommitted log at term 3.
///
/// ```
/// R0 ... 2,99 2,100
/// R1
/// R2 ... 3,99, 3,00
/// ```
///
/// - Start the cluster and node 2 start to replicate logs.
/// - test the log should be replicated to node 0.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_inconsistent_log() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut n_logs = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!("--- remove all nodes and fake the logs");

    let (r0, mut sto0) = router.remove_node(0).await.unwrap();
    let (r1, sto1) = router.remove_node(1).await.unwrap();
    let (r2, mut sto2) = router.remove_node(2).await.unwrap();

    r0.shutdown().await?;
    r1.shutdown().await?;
    r2.shutdown().await?;

    for i in n_logs + 1..=100 {
        sto0.append_to_log(&[&Entry {
            log_id: LogId::new(LeaderId::new(2, 0), i),
            payload: EntryPayload::Blank,
        }])
        .await?;

        sto2.append_to_log(&[&Entry {
            log_id: LogId::new(LeaderId::new(3, 0), i),
            payload: EntryPayload::Blank,
        }])
        .await?;
    }

    sto0.save_vote(&Vote {
        term: 2,
        node_id: 0,
        committed: false,
    })
    .await?;
    sto2.save_vote(&Vote {
        term: 3,
        node_id: 0,
        committed: false,
    })
    .await?;

    n_logs = 100;

    tracing::info!("--- restart node 1 and isolate. To let node-2 to become leader, node-1 should not vote for node-0");
    {
        router.new_raft_node_with_sto(1, sto1.clone()).await;
        router.isolate_node(1).await;
    }

    tracing::info!("--- restart node 0 and 2");
    {
        router.new_raft_node_with_sto(0, sto0.clone()).await;
        router.new_raft_node_with_sto(2, sto2.clone()).await;
    }

    // leader appends a blank log.
    n_logs += 1;

    tracing::info!("--- wait for node states");
    {
        router
            .wait_for_state(
                &btreeset! {0},
                State::Follower,
                Some(Duration::from_millis(2000)),
                "node 0 become follower",
            )
            .await?;

        router
            .wait_for_state(
                &btreeset! {2},
                State::Leader,
                Some(Duration::from_millis(5000)),
                "node 2 become leader",
            )
            .await?;
    }

    router
        .wait(&0, Some(Duration::from_millis(2000)))
        .await?
        .metrics(|x| x.last_log_index == Some(n_logs), "sync log to node 0")
        .await?;

    let logs = sto0.get_log_entries(60..=60).await?;
    assert_eq!(
        3,
        logs.first().unwrap().log_id.leader_id.term,
        "log is overridden by leader logs"
    );

    Ok(())
}
