use std::convert::TryInto;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::error::ChangeMembershipError;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::State;

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
    let mut router = RaftRouter::new(config.clone());

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
        router.add_learner(0, 1).await?;
        n_logs += 1;
        router.wait_for_log(&btreeset![0], Some(n_logs), timeout(), "add learner").await?;

        let node = router.get_raft_handle(&0)?;
        let res = node.change_membership(btreeset! {0,1}, true, false).await?;
        n_logs += 2;
        tracing::info!("--- change_membership blocks until success: {:?}", res);

        for node_id in 0..2 {
            let mut sto = router.get_storage_handle(&node_id)?;
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
    let mut router = RaftRouter::new(config.clone());

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

    tracing::info!("--- restore replication and change membership at once, expect NonVoterIsLagging");
    {
        router.restore_node(1).await;
        let node = router.get_raft_handle(&0)?;
        let res = node.change_membership(btreeset! {0,1}, false, false).await;

        tracing::info!("--- got res: {:?}", res);

        let err = res.unwrap_err();
        let err: ChangeMembershipError<memstore::Config> = err.try_into().unwrap();

        match err {
            ChangeMembershipError::LearnerIsLagging(e) => {
                tracing::info!(e.distance, "--- distance");
                assert_eq!(1, e.node_id);
                assert!(e.distance >= lag_threshold);
                assert!(e.distance < 500);
            }
            _ => {
                panic!("expect ChangeMembershipError::NonVoterNotFound");
            }
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn change_with_turn_not_exist_member_to_learner() -> anyhow::Result<()> {
    // Add a member without adding it as learner, in blocking mode it should finish successfully.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(Config { ..Default::default() }.validate()?);
    let mut router = RaftRouter::new(config.clone());
    let timeout = Some(Duration::from_millis(1000));
    let mut n_logs = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!("--- write up to 1 logs");
    {
        router.client_request_many(0, "non_voter_add", 1).await;
        n_logs += 1;

        // all the nodes MUST recv the log
        router.wait_for_log(&btreeset![0, 1, 2], Some(n_logs), timeout, "append a log").await?;
    }

    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(btreeset![0, 1], true, true).await?;
        // 2 for change_membership
        n_logs += 2;

        // all the nodes MUST recv the change_membership log
        router.wait_for_log(&btreeset![0, 1], Some(n_logs), timeout, "append a log").await?;
    }

    tracing::info!("--- write up to 1 logs");
    {
        router.client_request_many(0, "non_voter_add", 1).await;
        n_logs += 1;

        // node [0,1] MUST recv the log
        router.wait_for_log(&btreeset![0, 1], Some(n_logs), timeout, "append a log").await?;

        // node 2 MUST stay in learner state and is able to receive new logs
        router
            .wait_for_metrics(
                &2,
                |x| x.state == State::Learner,
                timeout,
                &format!("n{}.state -> {:?}", 2, State::Learner),
            )
            .await?;

        // node [2] MUST recv the log
        router.wait_for_log(&btreeset![2], Some(n_logs), timeout, "append a log").await?;

        // check membership
        router.wait_for_members(&btreeset![0, 1, 2], btreeset![0, 1], timeout, "members: [0,1]").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_micros(500))
}
