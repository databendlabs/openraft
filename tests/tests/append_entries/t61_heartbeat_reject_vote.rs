use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::VoteRequest;
use openraft::testing::log_id1;
use openraft::Config;
use openraft::Vote;
use tokio::time::sleep;
use tokio::time::Instant;

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

    let now = Instant::now();
    sleep(Duration::from_millis(1)).await;

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;

    let vote_modified_time = Arc::new(Mutex::new(Some(Instant::now())));
    tracing::info!(log_index, "--- leader lease is set by heartbeat");
    {
        let m = vote_modified_time.clone();

        router.external_request(1, move |state, _store, _net| {
            let mut l = m.lock().unwrap();
            *l = state.vote_last_modified().map(|t| t.into());
            assert!(state.vote_last_modified() > Some(now.into()));
        });

        let now = Instant::now();
        sleep(Duration::from_millis(700)).await;

        let m = vote_modified_time.clone();

        router.external_request(1, move |state, _store, _net| {
            let l = m.lock().unwrap();
            assert!(state.vote_last_modified() > Some(now.into()));
            assert!(state.vote_last_modified() > (*l).map(|t| t.into()));
        });
    }

    let node0 = router.get_raft_handle(&0)?;
    let node1 = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- leader lease rejects vote request");
    {
        let res = node1.vote(VoteRequest::new(Vote::new(10, 2), Some(log_id1(10, 10)))).await?;
        assert!(!res.vote_granted);
    }

    tracing::info!(log_index, "--- ensures no more blank-log heartbeat is used");
    {
        // TODO: this part can be removed when blank-log heartbeat is removed.
        sleep(Duration::from_millis(1500)).await;
        router.wait(&1, timeout()).log(Some(log_index), "no log is written").await?;
    }

    tracing::info!(log_index, "--- disable heartbeat, vote request will be granted");
    {
        node0.enable_heartbeat(false);
        sleep(Duration::from_millis(1500)).await;

        router.wait(&1, timeout()).log(Some(log_index), "no log is written").await?;

        let res = node1.vote(VoteRequest::new(Vote::new(10, 2), Some(log_id1(10, 10)))).await?;
        assert!(res.vote_granted, "vote is granted after leader lease expired");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
