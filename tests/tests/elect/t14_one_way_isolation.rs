use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Instant;
use openraft::ServerState;
use openraft::TokioInstant;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::Direction;
use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_error_type::RpcErrorType;
use crate::fixtures::ut_harness;

/// A one-way partition lets a follower receive heartbeats or send elections, but not both.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn one_way_isolation() -> Result<()> {
    let config = Config {
        enable_pre_vote: Some(false),
        election_timeout_min: 150,
        election_timeout_max: 151,
        ..Default::default()
    }
    .validate()?;
    let election_wait = Duration::from_millis(config.election_timeout_max * 2);
    let mut router = RaftRouter::new(Arc::new(config));
    router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;

    let vote = n1.metrics().borrow_watched().vote;
    let term = n1.metrics().borrow_watched().current_term;

    tracing::info!("--- node 1 receives heartbeats but cannot send RPCs");
    router.set_rpc_failure(1, Direction::NetSend, Some(RpcErrorType::NetworkError));
    let heartbeat_at = TokioInstant::now();
    n0.trigger().heartbeat().await?;
    wait_for_heartbeat(&router, heartbeat_at).await?;

    TypeConfig::sleep(election_wait).await;
    router
        .external_request(1, move |state| {
            assert_eq!(&vote, state.vote_ref());
            assert_eq!(ServerState::Follower, state.server_state);
        })
        .await?;

    tracing::info!("--- node 1 can send elections but cannot receive heartbeats");
    router.set_rpc_failure(1, Direction::NetSend, None);
    router.set_rpc_failure(1, Direction::NetRecv, Some(RpcErrorType::NetworkError));

    router
        .wait(&1, timeout())
        .metrics(|metrics| metrics.current_term > term, "node 1 starts an election")
        .await?;
    router
        .wait(&0, timeout())
        .leader_with_quorum_acked(None, "node 0 remains the healthy leader")
        .await?;

    tracing::info!("--- restore connectivity and wait for the cluster to converge");
    router.set_rpc_failure(1, Direction::NetRecv, None);
    let leader = router
        .wait(&1, timeout())
        .metrics(|metrics| metrics.current_leader.is_some(), "node 1 observes a leader")
        .await?
        .current_leader
        .unwrap();
    for node_id in [0, 1, 2] {
        router
            .wait(&node_id, timeout())
            .current_leader(leader, format!("node {} follows the converged leader", node_id))
            .await?;
    }

    Ok(())
}

async fn wait_for_heartbeat(router: &RaftRouter, after: TokioInstant) -> Result<()> {
    for _ in 0..20 {
        let last_modified = router.with_raft_state(1, |state| state.vote_last_modified()).await?;
        if last_modified > Some(after) {
            return Ok(());
        }

        TypeConfig::sleep(Duration::from_millis(50)).await;
    }

    anyhow::bail!("node 1 did not receive a heartbeat")
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
