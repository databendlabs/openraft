use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::alias::AsyncRuntimeOf;
use openraft::AsyncRuntime;
use openraft::Config;
use openraft_memstore::TypeConfig;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Get the last timestamp when a leader is acknowledged by a quorum,
/// from RaftMetrics and RaftServerMetrics.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn leader_last_ack_3_nodes() -> Result<()> {
    let heartbeat_interval = 50; // ms
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            heartbeat_interval,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let millis = n0.metrics().borrow().millis_since_quorum_ack;
    assert!(millis >= Some(0));

    {
        let millis = n0.data_metrics().borrow().millis_since_quorum_ack;
        assert!(millis >= Some(0));
    }

    tracing::info!(log_index, "--- sleep 500 ms, the `millis` should extend");
    {
        AsyncRuntimeOf::<TypeConfig>::sleep(Duration::from_millis(500)).await;

        let greater = n0.metrics().borrow().millis_since_quorum_ack;
        println!("greater: {:?}", greater);
        assert!(greater > millis);
        assert!(
            greater > Some(500 - heartbeat_interval * 2),
            "it extends, but may be smaller because the tick interval is 50 ms"
        );
    }

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- heartbeat; millis_since_quorum_ack refreshes");
    {
        n0.trigger().heartbeat().await?;
        n0.wait(timeout())
            .metrics(
                |x| x.millis_since_quorum_ack < Some(100),
                "millis_since_quorum_ack refreshed",
            )
            .await?;
    }

    tracing::info!(
        log_index,
        "--- sleep and heartbeat again; millis_since_quorum_ack refreshes"
    );
    {
        AsyncRuntimeOf::<TypeConfig>::sleep(Duration::from_millis(500)).await;

        n0.trigger().heartbeat().await?;

        n0.wait(timeout())
            .metrics(
                |x| x.millis_since_quorum_ack < Some(100),
                "millis_since_quorum_ack refreshed again",
            )
            .await?;
    }

    tracing::info!(log_index, "--- remove node 1 and node 2");
    {
        router.remove_node(1);
        router.remove_node(2);
    }

    tracing::info!(
        log_index,
        "--- sleep and heartbeat again; millis_since_quorum_ack does not refresh"
    );
    {
        AsyncRuntimeOf::<TypeConfig>::sleep(Duration::from_millis(500)).await;

        n0.trigger().heartbeat().await?;

        let got = n0
            .wait(timeout())
            .metrics(
                |x| x.millis_since_quorum_ack < Some(100),
                "millis_since_quorum_ack refreshed again",
            )
            .await;
        assert!(got.is_err(), "millis_since_quorum_ack does not refresh");
    }

    Ok(())
}

/// Get the last timestamp when a leader is acknowledged by a quorum,
/// from RaftMetrics and RaftServerMetrics.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn leader_last_ack_1_node() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;
    let _ = log_index;

    let n0 = router.get_raft_handle(&0)?;

    let millis = n0.metrics().borrow().millis_since_quorum_ack;
    assert_eq!(millis, Some(0), "it is always acked for single leader");

    {
        let millis = n0.data_metrics().borrow().millis_since_quorum_ack;
        assert_eq!(millis, Some(0), "it is always acked for single leader");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
