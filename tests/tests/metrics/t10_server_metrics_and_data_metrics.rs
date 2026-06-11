use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Membership;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;
#[allow(unused_imports)]
use pretty_assertions::assert_ne;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Server metrics and data metrics method should work.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn server_metrics_and_data_metrics() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let node = router.get_raft_handle(&0)?;
    let server_metrics = node.server_metrics();
    let data_metrics = node.data_metrics();

    let current_leader = router.current_leader(0).await;
    let server_metrics_1 = {
        let sm = server_metrics.borrow_watched();
        sm.clone()
    };
    let leader = server_metrics_1.current_leader;
    assert_eq!(leader, current_leader, "current_leader should be {:?}", current_leader);

    // Write some logs.
    let n = 10;
    tracing::info!(log_index, "--- write {} logs", n);
    log_index += router.client_request_many(0, "foo", n).await?;

    router.wait(&0, timeout()).applied_index(Some(log_index), "applied log index").await?;

    let last_log_index = data_metrics.borrow_watched().last_log.map(|x| x.index()).unwrap_or_default();
    assert_eq!(last_log_index, log_index, "last_log_index should be {:?}", log_index);

    let server_metrics_2 = server_metrics.borrow_watched().clone();

    // Server metrics should not change when only data (logs) are written.
    assert_eq!(server_metrics_1, server_metrics_2, "server metrics should not update");
    Ok(())
}

/// Test if heartbeat metrics work
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn heartbeat_metrics() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            heartbeat_interval: 50,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- trigger heartbeat; heartbeat metrics refreshes");
    let n1_hb_ts;
    let n2_hb_ts;
    {
        let now = TypeConfig::now();
        leader.trigger().heartbeat().await?;

        leader
            .wait(timeout())
            .metrics(
                |metrics| {
                    let heartbeat = metrics
                        .heartbeat
                        .as_ref()
                        .expect("expect heartbeat to be Some as metrics come from the leader node");
                    let node1 = heartbeat.get(&1);
                    let node2 = heartbeat.get(&2);

                    match (node1, node2) {
                        (Some(Some(node1)), Some(Some(node2))) => (**node1 >= now) && (**node2 >= now),
                        _ => false,
                    }
                },
                "millis_since_quorum_ack refreshed",
            )
            .await?;

        tracing::info!(
            log_index,
            "--- sleep 500 ms to drain all ongoing heartbeat notifications"
        );
        TypeConfig::sleep(Duration::from_millis(500)).await;

        let metrics = leader.metrics().borrow_watched().clone();
        let heartbeat = metrics
            .heartbeat
            .as_ref()
            .expect("expect heartbeat to be Some as metrics come from the leader node");
        n1_hb_ts = heartbeat.get(&1).unwrap().unwrap();
        n2_hb_ts = heartbeat.get(&2).unwrap().unwrap();
    }

    tracing::info!(log_index, "--- sleep 500 ms, the acked time should not change");
    {
        TypeConfig::sleep(Duration::from_millis(500)).await;

        let metrics = leader.metrics();
        let metrics_ref = metrics.borrow_watched();
        let heartbeat = metrics_ref
            .heartbeat
            .as_ref()
            .expect("expect heartbeat to be Some as metrics come from the leader node");

        let n1_hb_ts2 = heartbeat.get(&1).unwrap().unwrap();
        let n2_hb_ts2 = heartbeat.get(&2).unwrap().unwrap();

        assert_eq!(n1_hb_ts2, n1_hb_ts);
        assert_eq!(n2_hb_ts2, n2_hb_ts);
    }

    Ok(())
}

/// The `committed_membership_config` lags behind the effective `membership_config` until the
/// effective one is committed; the two become equal when a membership change is completed.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn committed_membership_config() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    let node = router.get_raft_handle(&0)?;

    let old_membership = Membership::new_with_defaults(vec![btreeset! {0,1}], []);
    let new_membership = Membership::new_with_defaults(vec![btreeset! {0,1}], [2]);

    tracing::info!(
        log_index,
        "--- the initial membership config is committed: the two are equal"
    );
    {
        let metrics = node.metrics().borrow_watched().clone();
        assert_eq!(metrics.committed_membership_config, metrics.membership_config);
        assert_eq!(&old_membership, metrics.committed_membership_config.membership());
    }

    tracing::info!(
        log_index,
        "--- block node-1, add a learner: the membership log can not commit"
    );
    router.new_raft_node(2).await;
    router.set_network_error(1, true);

    let n0 = node.clone();
    let add_learner_handle = TypeConfig::spawn(async move { n0.add_learner(2, (), false).await });

    tracing::info!(
        log_index,
        "--- the effective membership config updates, the committed one lags"
    );
    {
        let metrics = router
            .wait(&0, Some(Duration::from_millis(3_000)))
            .metrics(
                |x| x.membership_config.membership() == &new_membership,
                "the effective membership config contains the new learner",
            )
            .await?;

        assert_eq!(&old_membership, metrics.committed_membership_config.membership());
        assert!(metrics.committed_membership_config.log_id() < metrics.membership_config.log_id());
    }

    tracing::info!(
        log_index,
        "--- restore node-1, the membership log commits: the two are equal"
    );
    {
        router.set_network_error(1, false);
        add_learner_handle.await??;

        let metrics = router
            .wait(&0, Some(Duration::from_millis(3_000)))
            .metrics(
                |x| x.committed_membership_config == x.membership_config,
                "the committed membership config catches up with the effective one",
            )
            .await?;
        assert_eq!(&new_membership, metrics.committed_membership_config.membership());

        let server_metrics = node.server_metrics().borrow_watched().clone();
        assert_eq!(
            metrics.committed_membership_config,
            server_metrics.committed_membership_config
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
