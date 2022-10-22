use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::StorageHelper;

use crate::fixtures::RaftRouter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn change_with_new_learner_blocking() -> anyhow::Result<()> {
    // Add a member without adding it as learner, in blocking mode it should finish successfully.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let lag_threshold = 1;

    let config = Arc::new(
        Config {
            replication_lag_threshold: lag_threshold,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- write up to 100 logs");
    {
        router.client_request_many(0, "non_voter_add", 100 - n_logs as usize).await;
        n_logs = 100;

        router.wait(&0, timeout()).await?.log(Some(n_logs), "received 100 logs").await?;
    }

    tracing::info!("--- change membership without adding-learner");
    {
        router.new_raft_node(1).await;

        let res = router.change_membership_with_blocking(0, btreeset! {0,1}, true).await?;
        n_logs += 2;
        tracing::info!("--- change_membership blocks until success: {:?}", res);

        for node_id in 0..2 {
            let sto = router.get_storage_handle(&node_id).await?;
            let logs = StorageHelper::new(&sto).get_log_entries(..).await?;
            assert_eq!(n_logs, logs[logs.len() - 1].log_id.index, "node: {}", node_id);
            // 0-th log
            assert_eq!(n_logs + 1, logs.len() as u64, "node: {}", node_id);
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn change_with_lagging_learner_non_blocking() -> anyhow::Result<()> {
    // Add a learner into membership config.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let lag_threshold = 1;

    let config = Arc::new(
        Config {
            replication_lag_threshold: lag_threshold,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!("--- stop replication by isolating node 1");
    {
        router.isolate_node(1).await;
    }

    tracing::info!("--- write up to 100 logs");
    {
        router.client_request_many(0, "non_voter_add", 500 - log_index as usize).await;
        log_index = 500;

        router.wait(&0, timeout()).await?.log(Some(log_index), "received 500 logs").await?;
    }

    tracing::info!(
        "--- restore replication and change membership at once, it still blocks until logs are replicated to node-1"
    );
    {
        router.restore_node(1).await;
        let res = router.change_membership_with_blocking(0, btreeset! {0,1}, false).await;
        log_index += 2;

        tracing::info!("--- got res: {:?}", res);
        assert!(res.is_ok());
        router.wait(&1, timeout()).await?.log(Some(log_index), "received 500+2 logs").await?;
    }

    tracing::info!("--- add node-3, with blocking=false, won't block, because [0,1] is a quorum");
    {
        router.new_raft_node(2).await;

        router.change_membership_with_blocking(0, btreeset! {0,1,2}, false).await?;
        log_index += 2;
        let m = router.get_metrics(&2).await?;
        assert!(m.last_log_index < Some(log_index));
    }

    tracing::info!("--- make sure replication to node-3 works as expected");
    {
        router
            .wait(&2, timeout())
            .await?
            .log(Some(log_index), "received all logs, replication works")
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
