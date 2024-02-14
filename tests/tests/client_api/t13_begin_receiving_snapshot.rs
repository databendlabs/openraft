use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::error::HigherVote;
use openraft::Config;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn begin_receiving_snapshot() -> anyhow::Result<()> {
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

    tracing::info!(log_index, "--- isolate node 2 so that it can receive snapshot");
    router.set_unreachable(2, true);

    tracing::info!(log_index, "--- write to make node-0,1 have more logs");
    {
        log_index += router.client_request_many(0, "foo", 3).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "write more log").await?;
        router.wait(&1, timeout()).applied_index(Some(log_index), "write more log").await?;
    }

    tracing::info!(log_index, "--- fails to execute with smaller vote");
    {
        let n1 = router.get_raft_handle(&1)?;

        let res = n1.begin_receiving_snapshot(Vote::new(0, 0)).await;
        assert_eq!(
            HigherVote {
                higher: Vote::new_committed(1, 0),
                mine: Vote::new(0, 0),
            },
            res.unwrap_err().into_api_error().unwrap()
        );
    }

    tracing::info!(log_index, "--- got a snapshot data");
    {
        let n1 = router.get_raft_handle(&1)?;
        let _resp = n1.begin_receiving_snapshot(Vote::new_committed(1, 0)).await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
