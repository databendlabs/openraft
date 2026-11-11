use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::TickDriver;
use openraft::TickSourceError;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Fatal;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_rule::RpcRule;
use crate::fixtures::ut_harness;

fn timer_config() -> Config {
    Config {
        heartbeat_interval: 40,
        election_timeout_min: 80,
        election_timeout_max: 120,
        ..Default::default()
    }
}

fn kind_count(router: &RaftRouter, kind: RpcKind) -> u64 {
    let counts = router.get_rpc_kind_count();
    counts.get(&kind).copied().unwrap_or(0)
}

async fn wait_kind(router: &RaftRouter, kind: RpcKind, expected: u64) -> Result<()> {
    let start = TypeConfig::now();
    loop {
        let count = kind_count(router, kind);
        if count == expected {
            return Ok(());
        }
        let elapsed = TypeConfig::now() - start;
        if elapsed > Duration::from_secs(2) {
            anyhow::bail!("{kind:?} count is {count}, expected {expected}");
        }
        TypeConfig::sleep(Duration::from_millis(10)).await;
    }
}

fn disable_ticks(router: &RaftRouter, ids: &[u64]) -> Result<()> {
    for id in ids {
        let node = router.get_raft_handle(id)?;
        node.runtime_config().tick(false);
    }
    Ok(())
}

/// An external tick checks the current deadlines once.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_heartbeat_is_once() -> Result<()> {
    let config = timer_config();
    let mut router = RaftRouter::new(config)?;
    router.new_cluster([0, 1], []).await?;
    disable_ticks(&router, &[0, 1])?;

    tracing::info!("--- wait until the leader heartbeat deadline is in the past");
    {
        TypeConfig::sleep(Duration::from_millis(60)).await;
    }

    tracing::info!("--- one due tick sends one heartbeat and leaves the vote unchanged");
    let vote_before = {
        let leader = router.get_raft_handle(&0)?;
        let metrics = leader.metrics().borrow_watched().clone();
        let before = kind_count(&router, RpcKind::AppendEntries);
        leader.trigger().tick().await?;
        wait_kind(&router, RpcKind::AppendEntries, before + 1).await?;
        let after = leader.metrics().borrow_watched().clone();
        assert_eq!(metrics.vote, after.vote);
        assert_eq!(metrics.current_term, after.current_term);
        metrics.vote
    };

    tracing::info!("--- an immediate second tick sends nothing");
    {
        let leader = router.get_raft_handle(&0)?;
        let before = kind_count(&router, RpcKind::AppendEntries);
        leader.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(15)).await;
        let after = kind_count(&router, RpcKind::AppendEntries);
        assert_eq!(before, after);
        let metrics = leader.metrics();
        let watched = metrics.borrow_watched();
        let vote = watched.vote;
        assert_eq!(vote_before, vote);
    }

    tracing::info!("--- a tick after several missed intervals still sends one heartbeat");
    {
        TypeConfig::sleep(Duration::from_millis(200)).await;
        let leader = router.get_raft_handle(&0)?;
        let before = kind_count(&router, RpcKind::AppendEntries);
        leader.trigger().tick().await?;
        wait_kind(&router, RpcKind::AppendEntries, before + 1).await?;
        TypeConfig::sleep(Duration::from_millis(15)).await;
        let after = kind_count(&router, RpcKind::AppendEntries);
        assert_eq!(before + 1, after);
    }

    Ok(())
}

/// Disabled heartbeat and election flags suppress the corresponding timer work.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_honors_heartbeat_and_elect_flags() -> Result<()> {
    let config = timer_config();
    let mut router = RaftRouter::new(config)?;
    router.new_cluster([0, 1], []).await?;
    disable_ticks(&router, &[0, 1])?;

    tracing::info!("--- a due tick sends no heartbeat while heartbeat is disabled");
    {
        let leader = router.get_raft_handle(&0)?;
        leader.runtime_config().heartbeat(false);
        TypeConfig::sleep(Duration::from_millis(60)).await;
        let before = kind_count(&router, RpcKind::AppendEntries);
        leader.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(40)).await;
        let after = kind_count(&router, RpcKind::AppendEntries);
        assert_eq!(before, after);
    }

    tracing::info!("--- an expired follower does not campaign while election is disabled");
    let vote_before = {
        let follower = router.get_raft_handle(&1)?;
        follower.runtime_config().elect(false);
        TypeConfig::sleep(Duration::from_millis(300)).await;
        let metrics = follower.metrics().borrow_watched().clone();
        let votes_before = kind_count(&router, RpcKind::Vote);
        follower.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(40)).await;
        let votes_after = kind_count(&router, RpcKind::Vote);
        assert_eq!(votes_before, votes_after);
        let after = follower.metrics().borrow_watched().clone();
        assert_eq!(metrics.vote, after.vote);
        assert_eq!(metrics.current_term, after.current_term);
        metrics
    };

    tracing::info!("--- enabling election lets the same due tick campaign");
    {
        let follower = router.get_raft_handle(&1)?;
        follower.runtime_config().elect(true);
        follower.trigger().tick().await?;
        router.wait(&1).state(ServerState::Leader, "follower elected by an external tick").await?;
        let metrics = follower.metrics().borrow_watched().clone();
        let expected = VoteOf::<TypeConfig>::new_committed(vote_before.current_term + 1, 1);
        assert_eq!(expected, metrics.vote);
    }

    Ok(())
}

/// A learner does not campaign when it receives a tick.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_leaves_a_learner() -> Result<()> {
    let config = timer_config();
    let mut router = RaftRouter::new(config)?;
    router.new_cluster([0, 1], [2]).await?;
    disable_ticks(&router, &[0, 1, 2])?;

    tracing::info!("--- a tick after the election timeout leaves the learner in place");
    {
        TypeConfig::sleep(Duration::from_millis(300)).await;
        let learner = router.get_raft_handle(&2)?;
        let before = learner.metrics().borrow_watched().clone();
        learner.trigger().tick().await?;
        TypeConfig::sleep(Duration::from_millis(40)).await;
        let after = learner.metrics().borrow_watched().clone();
        assert_eq!(before.vote, after.vote);
        assert_eq!(before.current_term, after.current_term);
        assert_eq!(ServerState::Learner, after.state);
    }

    Ok(())
}

/// A due tick with Pre-Vote enabled probes without changing the term.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_pre_vote_does_not_change_term() -> Result<()> {
    let config = Config {
        enable_pre_vote: Some(true),
        ..timer_config()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_cluster([0, 1, 2], []).await?;
    disable_ticks(&router, &[1])?;
    router.add_rpc_rule(RpcRule::new(0, 1, RpcKind::AppendEntries));

    tracing::info!("--- node 1 loses the leader lease while the leader still has a quorum");
    {
        TypeConfig::sleep(Duration::from_millis(300)).await;
    }

    tracing::info!("--- the due tick sends Pre-Vote and leaves the term and vote unchanged");
    {
        let follower = router.get_raft_handle(&1)?;
        let before = follower.metrics().borrow_watched().clone();
        let pre_before = kind_count(&router, RpcKind::PreVote);
        let vote_before = kind_count(&router, RpcKind::Vote);
        follower.trigger().tick().await?;
        wait_kind(&router, RpcKind::PreVote, pre_before + 2).await?;
        TypeConfig::sleep(Duration::from_millis(40)).await;
        let pre_after = kind_count(&router, RpcKind::PreVote);
        let vote_after = kind_count(&router, RpcKind::Vote);
        assert_eq!(pre_before + 2, pre_after);
        assert_eq!(vote_before, vote_after);
        let after = follower.metrics().borrow_watched().clone();
        assert_eq!(before.vote, after.vote);
        assert_eq!(before.current_term, after.current_term);
    }

    Ok(())
}

/// A stopped core rejects a tick, and a running core rejects a late driver selection.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_tick_shutdown_and_driver_after_start() -> Result<()> {
    let config = timer_config();
    let mut router = RaftRouter::new(config)?;
    router.new_cluster([0], []).await?;

    tracing::info!("--- use_tick_driver refuses a node whose core has started");
    {
        let node = router.get_raft_handle(&0)?;
        let driver = TickDriver::<TypeConfig>::spawn();
        let result = node.use_tick_driver(&driver);
        assert_eq!(Err(TickSourceError::AlreadyStarted), result);
        driver.shutdown().await?;
    }

    tracing::info!("--- tick after shutdown returns the stopped fatal error");
    {
        let node = router.get_raft_handle(&0)?;
        node.shutdown().await?;
        let result = node.trigger().tick().await;
        assert_eq!(Err(Fatal::Stopped), result);
    }

    Ok(())
}
