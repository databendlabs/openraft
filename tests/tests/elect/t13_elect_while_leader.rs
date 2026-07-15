use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Triggering an election on the current Leader is ignored.
///
/// A Leader can not win a campaign it starts: its own heartbeats keep refreshing the voters' leader
/// lease, and `handle_vote_req()` rejects a vote request while the lease has not expired. Entering
/// Candidate would therefore only strip the node of leadership and inflate the term for as long as
/// it keeps heartbeating, leaving the group without a Leader.
///
/// Instead the trigger is a no-op: the established leadership is left alone.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn elect_on_leader_is_ignored() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: true,
            enable_elect: true,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is the initial leader").await?;

    let term_before = n0.metrics().borrow_watched().current_term;

    tracing::info!("--- trigger an election on the current leader, with and without Pre-Vote");
    n0.trigger().elect(false).await?;
    n0.trigger().elect(true).await?;

    TypeConfig::sleep(Duration::from_millis(500)).await;

    tracing::info!("--- node 0 is still the leader of the same term");
    {
        let m = n0.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, m.state, "node 0 remains a Leader");
        assert_eq!(term_before, m.current_term, "the term is not inflated");
        assert_eq!(Some(0), m.current_leader, "node 0 remains the leader");
    }

    tracing::info!("--- the followers still see node 0 as the leader");
    for id in [1, 2] {
        router
            .get_raft_handle(&id)?
            .wait(timeout())
            .metrics(|m| m.current_leader == Some(0), "follower still follows node 0")
            .await?;
    }

    tracing::info!("--- and the leader still serves writes");
    router.client_request_many(0, "foo", 1).await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
