use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Triggering an election on the current leader must not livelock the cluster.
///
/// When a node that is already a leader starts a new election, it must relinquish its
/// leadership: stop the heartbeat and replication workers of the old term. Otherwise it becomes
/// a Candidate for the new term while its stale heartbeats keep carrying the *old* committed
/// vote. Those heartbeats keep refreshing the followers' leader lease, so the followers reject
/// the Candidate's own (pre-)vote requests ("leader lease has not yet expired") for as long as
/// the node keeps heartbeating. The node can never win, the followers never time out, and the
/// group stays leaderless forever.
///
/// With leadership correctly relinquished, the stale heartbeats stop, the followers' lease
/// expires, and a new leader is elected.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn elect_on_leader_does_not_livelock() -> Result<()> {
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

    tracing::info!("--- trigger a fresh election on the current leader");
    n0.trigger().elect(false).await?;

    tracing::info!("--- node 0 leaves the leader state to campaign for the new term");
    n0.wait(timeout()).state(ServerState::Candidate, "node 0 becomes a candidate").await?;

    tracing::info!("--- the cluster elects a leader again; a livelock would leave node 0 leaderless");
    n0.wait(Some(Duration::from_secs(5)))
        .metrics(|m| m.current_leader.is_some(), "an established leader emerges")
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
