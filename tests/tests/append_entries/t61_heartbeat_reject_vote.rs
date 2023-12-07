use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::VoteRequest;
use openraft::testing::log_id;
use openraft::Config;
use openraft::TokioInstant;
use openraft::Vote;
use tokio::time::sleep;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// If a follower receives heartbeat, it should reject vote request until leader lease expired.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn heartbeat_reject_vote() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 200,
            election_timeout_min: 1000,
            election_timeout_max: 1001,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let now = TokioInstant::now();
    sleep(Duration::from_millis(1)).await;

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;

    let vote_modified_time = Arc::new(Mutex::new(Some(TokioInstant::now())));
    tracing::info!(log_index, "--- leader lease is set by heartbeat");
    {
        let m = vote_modified_time.clone();

        router.external_request(1, move |state| {
            let mut l = m.lock().unwrap();
            *l = state.vote_last_modified();
            assert!(state.vote_last_modified() > Some(now));
        });

        let now = TokioInstant::now();
        sleep(Duration::from_millis(700)).await;

        let m = vote_modified_time.clone();

        router.external_request(1, move |state| {
            let l = m.lock().unwrap();
            assert!(state.vote_last_modified() > Some(now));
            assert!(state.vote_last_modified() > *l);
        });
    }

    let node0 = router.get_raft_handle(&0)?;
    let node1 = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- leader lease rejects vote request");
    {
        let res = node1.vote(VoteRequest::new(Vote::new(10, 2), Some(log_id(10, 1, 10)))).await?;
        assert!(!res.vote_granted);
    }

    tracing::info!(log_index, "--- ensures no more blank-log heartbeat is used");
    {
        // TODO: this part can be removed when blank-log heartbeat is removed.
        sleep(Duration::from_millis(1500)).await;
        router.wait(&1, timeout()).applied_index(Some(log_index), "no log is written").await?;
    }

    tracing::info!(log_index, "--- disable heartbeat, vote request will be granted");
    {
        node0.runtime_config().heartbeat(false);
        sleep(Duration::from_millis(1500)).await;

        router.wait(&1, timeout()).applied_index(Some(log_index), "no log is written").await?;

        let res = node1.vote(VoteRequest::new(Vote::new(10, 2), Some(log_id(10, 1, 10)))).await?;
        assert!(res.vote_granted, "vote is granted after leader lease expired");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
