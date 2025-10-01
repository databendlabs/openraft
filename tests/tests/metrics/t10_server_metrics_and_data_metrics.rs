use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
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
    let mut server_metrics = node.server_metrics();
    let data_metrics = node.data_metrics();

    let current_leader = router.current_leader(0).await;
    let server_metrics_1 = {
        let sm = server_metrics.borrow_and_update();
        sm.clone()
    };
    let leader = server_metrics_1.current_leader;
    assert_eq!(leader, current_leader, "current_leader should be {:?}", current_leader);

    // Write some logs.
    let n = 10;
    tracing::info!(log_index, "--- write {} logs", n);
    log_index += router.client_request_many(0, "foo", n).await?;

    router.wait(&0, timeout()).applied_index(Some(log_index), "applied log index").await?;

    let last_log_index = data_metrics.borrow().last_log.unwrap_or_default().index();
    assert_eq!(last_log_index, log_index, "last_log_index should be {:?}", log_index);

    let sm = server_metrics.borrow();
    let server_metrics_2 = sm.clone();

    // TODO: flaky fail, find out why.
    assert!(
        !sm.has_changed(),
        "server metrics should not update, but {:?} --> {:?}",
        server_metrics_1,
        server_metrics_2
    );
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
    let refreshed_node1;
    let refreshed_node2;
    {
        let now = TypeConfig::now();
        leader.trigger().heartbeat().await?;

        let metrics = leader
            .wait(timeout())
            .metrics(
                |metrics| {
                    let heartbeat = metrics
                        .heartbeat
                        .as_ref()
                        .expect("expect heartbeat to be Some as metrics come from the leader node");
                    let node1 = heartbeat.get(&1).unwrap().unwrap();
                    let node2 = heartbeat.get(&2).unwrap().unwrap();

                    (*node1 >= now) && (*node2 >= now)
                },
                "millis_since_quorum_ack refreshed",
            )
            .await?;

        let heartbeat = metrics
            .heartbeat
            .as_ref()
            .expect("expect heartbeat to be Some as metrics come from the leader node");
        refreshed_node1 = heartbeat.get(&1).unwrap().unwrap();
        refreshed_node2 = heartbeat.get(&2).unwrap().unwrap();
    }

    tracing::info!(log_index, "--- sleep 500 ms, the acked time should not change");
    {
        TypeConfig::sleep(Duration::from_millis(500)).await;

        let metrics = leader.metrics();
        let metrics_ref = metrics.borrow();
        let heartbeat = metrics_ref
            .heartbeat
            .as_ref()
            .expect("expect heartbeat to be Some as metrics come from the leader node");

        let got_node1 = heartbeat.get(&1).unwrap().unwrap();
        let got_node2 = heartbeat.get(&2).unwrap().unwrap();

        assert!(got_node1 == refreshed_node1);
        assert!(got_node2 == refreshed_node2);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
