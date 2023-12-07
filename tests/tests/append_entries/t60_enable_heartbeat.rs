use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::AsyncRuntime;
use openraft::Config;
use openraft::TokioRuntime;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Enable heartbeat, heartbeat should be replicated.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn enable_heartbeat() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let _ = log_index;

    let node0 = router.get_raft_handle(&0)?;
    node0.runtime_config().heartbeat(true);

    for _i in 0..3 {
        let now = <TokioRuntime as AsyncRuntime>::Instant::now();
        TokioRuntime::sleep(Duration::from_millis(500)).await;

        for node_id in [1, 2, 3] {
            // no new log will be sent, .
            router
                .wait(&node_id, timeout())
                .applied_index_at_least(Some(log_index), format!("node {} emit heartbeat log", node_id))
                .await?;

            // leader lease is extended.
            router.external_request(node_id, move |state| {
                assert!(state.vote_last_modified() > Some(now));
            });
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
