use std::convert::TryInto;
use std::sync::Arc;
use std::time::Duration;

use async_raft::ChangeMembershipError;
use async_raft::Config;
use async_raft::RaftStorage;
use maplit::btreeset;

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

        router.wait(&0, timeout()).await?.log(n_logs, "received 100 logs").await?;
    }

    tracing::info!("--- change membership without adding-learner");
    {
        router.new_raft_node(1).await;

        let res = router.change_membership_with_blocking(0, btreeset! {0,1}, true).await?;
        n_logs += 2;
        tracing::info!("--- change_membership blocks until success: {:?}", res);

        for node_id in 0..2 {
            let sto = router.get_storage_handle(&node_id).await?;
            let logs = sto.get_log_entries(..).await?;
            assert_eq!(n_logs, logs[logs.len() - 1].log_id.index, "node: {}", node_id);
            // 0-th log
            assert_eq!(n_logs + 1, logs.len() as u64, "node: {}", node_id);
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn change_with_lagging_learner_non_blocking() -> anyhow::Result<()> {
    // Add a learner into membership config, expect error NonVoterIsLagging.

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

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!("--- stop replication by isolating node 1");
    {
        router.isolate_node(1).await;
    }

    tracing::info!("--- write up to 100 logs");
    {
        router.client_request_many(0, "non_voter_add", 500 - n_logs as usize).await;
        n_logs = 500;

        router.wait(&0, timeout()).await?.log(n_logs, "received 500 logs").await?;
    }

    tracing::info!("--- restore replication and change membership at once, expect NonVoterIsLagging");
    {
        router.restore_node(1).await;
        let res = router.change_membership_with_blocking(0, btreeset! {0,1}, false).await;

        tracing::info!("--- got res: {:?}", res);

        let err = res.unwrap_err();
        let err: ChangeMembershipError = err.try_into().unwrap();

        match err {
            ChangeMembershipError::LearnerIsLagging {
                node_id,
                matched: _,
                distance,
            } => {
                tracing::info!(distance, "--- distance");
                assert_eq!(1, node_id);
                assert!(distance >= lag_threshold);
                assert!(distance < 500);
            }
            _ => {
                panic!("expect ChangeMembershipError::NonVoterNotFound");
            }
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_micros(500))
}
