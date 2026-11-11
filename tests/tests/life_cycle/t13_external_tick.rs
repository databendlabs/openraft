use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::RaftMetrics;
use openraft::ServerState;
use openraft::Vote;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Fatal;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::InstantOf;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

fn rpc_count(router: &RaftRouter, kind: RpcKind) -> u64 {
    router.get_rpc_kind_count().get(&kind).copied().unwrap_or(0)
}

/// Whether the leader has heartbeat acks from nodes 1 and 2 received at or after `since`.
fn followers_acked_since(metrics: &RaftMetrics<TypeConfig>, since: InstantOf<TypeConfig>) -> bool {
    let Some(heartbeat) = metrics.heartbeat.as_ref() else {
        return false;
    };
    [1, 2].iter().all(|id| matches!(heartbeat.get(id), Some(Some(acked)) if **acked >= since))
}

/// With the internal ticker disabled, `Trigger::tick()` sends a heartbeat only when it is due.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_sends_due_heartbeat() -> Result<()> {
    let config = Config {
        enable_tick: false,
        heartbeat_interval: 500,
        election_timeout_min: 1000,
        election_timeout_max: 1001,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- a new leader's heartbeat is due at once: one tick sends one heartbeat to each follower"
    );
    {
        let before = rpc_count(&router, RpcKind::AppendEntries);
        let now = TypeConfig::now();
        leader.trigger().tick().await?;
        leader
            .wait(timeout())
            .metrics(|m| followers_acked_since(m, now), "followers acked the heartbeat")
            .await?;
        assert_eq!(before + 2, rpc_count(&router, RpcKind::AppendEntries));
    }

    tracing::info!(
        log_index,
        "--- a tick before heartbeat_interval has passed sends nothing"
    );
    {
        let before = rpc_count(&router, RpcKind::AppendEntries);
        leader.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(100)).await;
        assert_eq!(before, rpc_count(&router, RpcKind::AppendEntries));
    }

    tracing::info!(
        log_index,
        "--- after two missed intervals, one tick sends one heartbeat, not one per missed interval"
    );
    {
        TypeConfig::sleep(Duration::from_millis(1100)).await;
        let before = rpc_count(&router, RpcKind::AppendEntries);
        let now = TypeConfig::now();
        leader.trigger().tick().await?;
        leader
            .wait(timeout())
            .metrics(|m| followers_acked_since(m, now), "followers acked the heartbeat")
            .await?;
        TypeConfig::sleep(Duration::from_millis(100)).await;
        assert_eq!(before + 2, rpc_count(&router, RpcKind::AppendEntries));
    }

    Ok(())
}

/// With the internal ticker disabled, `Trigger::tick()` starts an election only after the
/// election timeout expires, and only while election is enabled.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_starts_election_after_timeout() -> Result<()> {
    let config = Config {
        enable_tick: false,
        heartbeat_interval: 100,
        election_timeout_min: 500,
        election_timeout_max: 501,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n1 = router.get_raft_handle(&1)?;
    let (vote_before, term_before) = {
        let metrics = n1.metrics().borrow_watched().clone();
        (metrics.vote, metrics.current_term)
    };
    let election_rpcs = |router: &RaftRouter| rpc_count(router, RpcKind::Vote) + rpc_count(router, RpcKind::PreVote);

    tracing::info!(
        log_index,
        "--- a tick before the election timeout leaves node 1 a follower"
    );
    {
        let before = election_rpcs(&router);
        n1.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(100)).await;

        let metrics = n1.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Follower, metrics.state);
        assert_eq!(vote_before, metrics.vote);
        assert_eq!(before, election_rpcs(&router));
    }

    tracing::info!(
        log_index,
        "--- no heartbeat arrives, so node 1's lease expires; with election disabled a tick does not campaign"
    );
    {
        TypeConfig::sleep(Duration::from_millis(1100)).await;
        n1.runtime_config().elect(false);

        let before = election_rpcs(&router);
        n1.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(100)).await;

        let metrics = n1.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Follower, metrics.state);
        assert_eq!(vote_before, metrics.vote);
        assert_eq!(before, election_rpcs(&router));
    }

    tracing::info!(
        log_index,
        "--- with election enabled again, the next tick elects node 1"
    );
    {
        n1.runtime_config().elect(true);
        n1.trigger().tick().await?;
        n1.wait(timeout()).state(ServerState::Leader, "node 1 elected by an external tick").await?;

        let vote = n1.metrics().borrow_watched().vote;
        assert_eq!(Vote::new_committed(term_before + 1, 1), vote);
    }

    Ok(())
}

/// `Trigger::tick()` on a stopped node returns `Fatal::Stopped`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_after_shutdown() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- shut down node 0, then tick it");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.shutdown().await?;
        let result = n0.trigger().tick().await;
        assert_eq!(Err(Fatal::Stopped), result);
    }

    Ok(())
}
