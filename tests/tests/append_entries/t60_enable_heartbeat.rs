use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::impls::TokioRuntime;
use openraft::type_config::AsyncRuntime;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Enable heartbeat, heartbeat should be replicated.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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
        let now = TypeConfig::now();
        TokioRuntime::sleep(Duration::from_millis(500)).await;

        for node_id in [1, 2, 3] {
            // no new log will be sent, .
            router
                .wait(&node_id, timeout())
                .applied_index_at_least(Some(log_index), format!("node {} emit heartbeat log", node_id))
                .await?;

            // leader lease is extended.
            router
                .external_request(node_id, move |state| {
                    assert!(state.vote_last_modified() > Some(now));
                })
                .await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
