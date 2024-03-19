use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Server metrics and data metrics method should work.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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

    let last_log_index = data_metrics.borrow().last_log.unwrap_or_default().index;
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

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
