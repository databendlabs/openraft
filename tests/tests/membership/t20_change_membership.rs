use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::error::ChangeMembershipError;
use openraft::error::ClientWriteError;
use openraft::storage::RaftLogReaderExt;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// When a change-membership log is committed, the `RaftState.membership_state` should be updated.
#[async_entry::test(worker_threads = 3, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn update_membership_state() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3,4}).await?;

    tracing::info!(log_index, "--- change membership from 012 to 01234");
    {
        let leader = router.get_raft_handle(&0)?;
        let res = leader.change_membership([0, 1, 2, 3, 4], false).await?;
        log_index += 2;

        tracing::info!(log_index, "--- change_membership blocks until success: {:?}", res);

        for node_id in [0, 1, 2, 3, 4] {
            router
                .wait(&node_id, timeout())
                .applied_index(Some(log_index), "change-membership log applied")
                .await?;
            router.external_request(node_id, move |st| {
                tracing::debug!("--- got state: {:?}", st);
                assert_eq!(st.membership_state.committed().log_id().index(), Some(log_index));
                assert_eq!(st.membership_state.effective().log_id().index(), Some(log_index));
            });
        }
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn change_with_new_learner_blocking() -> anyhow::Result<()> {
    // Add a member without adding it as learner, in blocking mode it should finish successfully.

    let lag_threshold = 1;

    let config = Arc::new(
        Config {
            replication_lag_threshold: lag_threshold,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write up to 100 logs");
    {
        router.client_request_many(0, "non_voter_add", 100 - log_index as usize).await?;
        log_index = 100;

        router.wait(&0, timeout()).applied_index(Some(log_index), "received 100 logs").await?;
    }

    tracing::info!(log_index, "--- change membership without adding-learner");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await?;
        log_index += 1;
        router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "add learner").await?;

        let node = router.get_raft_handle(&0)?;
        let res = node.change_membership([0, 1], false).await?;
        log_index += 2;
        tracing::info!(log_index, "--- change_membership blocks until success: {:?}", res);

        for node_id in 0..2 {
            let (mut sto, _sm) = router.get_storage_handle(&node_id)?;
            let logs = sto.get_log_entries(..).await?;
            assert_eq!(log_index, logs[logs.len() - 1].log_id.index, "node: {}", node_id);
            // 0-th log
            assert_eq!(log_index + 1, logs.len() as u64, "node: {}", node_id);
        }
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn change_without_adding_learner() -> anyhow::Result<()> {
    let config = Arc::new(Config { ..Default::default() }.validate()?);
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "received 100 logs").await?;

    router.new_raft_node(1).await;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- change membership without adding-learner, allow_lagging=true"
    );
    {
        let res = leader.change_membership([0, 1], false).await;
        let raft_err = res.unwrap_err();
        tracing::debug!("raft_err: {:?}", raft_err);

        match raft_err.api_error().unwrap() {
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::LearnerNotFound(err)) => {
                assert_eq!(1, err.node_id);
            }
            _ => {
                unreachable!("expect LearnerNotFound")
            }
        }
    }

    tracing::info!(log_index, "--- change membership without adding-learner");
    {
        let res = leader.change_membership([0, 1], false).await;
        let raft_err = res.unwrap_err();
        match raft_err.api_error().unwrap() {
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::LearnerNotFound(err)) => {
                assert_eq!(1, err.node_id);
            }
            _ => {
                unreachable!("expect LearnerNotFound")
            }
        }
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn change_with_turn_removed_voter_to_learner() -> anyhow::Result<()> {
    // Add a member without adding it as learner, in blocking mode it should finish successfully.

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    let timeout = Some(Duration::from_millis(1000));
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write up to 1 logs");
    {
        router.client_request_many(0, "non_voter_add", 1).await?;
        log_index += 1;

        // all the nodes MUST recv the log
        router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), timeout, "append a log").await?;
    }

    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([0, 1], true).await?;
        // 2 for change_membership
        log_index += 2;

        // all the nodes MUST recv the change_membership log
        router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout, "append a log").await?;
    }

    tracing::info!(log_index, "--- write up to 1 logs");
    {
        router.client_request_many(0, "non_voter_add", 1).await?;
        log_index += 1;

        // node [0,1] MUST recv the log
        router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout, "append a log").await?;

        // node 2 MUST stay in learner state and is able to receive new logs
        router
            .wait_for_metrics(
                &2,
                |x| x.state == ServerState::Learner,
                timeout,
                &format!("n{}.state -> {:?}", 2, ServerState::Learner),
            )
            .await?;

        // node [2] MUST recv the log
        router.wait_for_log(&btreeset![2], Some(log_index), timeout, "append a log").await?;

        // check membership
        router.wait_for_members(&btreeset![0, 1, 2], btreeset![0, 1], timeout, "members: [0,1]").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
