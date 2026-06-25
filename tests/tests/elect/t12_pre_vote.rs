use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::Vote;
use openraft::async_runtime::WatchReceiver;
use openraft::raft::VoteRequest;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// With Pre-Vote enabled, an isolated follower does not inflate its term.
///
/// This is the scenario from databendlabs/openraft#1770: a follower that loses contact with the
/// leader keeps timing out. Without Pre-Vote it bumps its term every timeout (1 → 2 → 3 → ...),
/// and that inflated term disrupts the healthy leader once the follower reconnects. With Pre-Vote
/// the follower cannot reach a quorum to grant its Pre-Vote, so it never increments its term.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn pre_vote_prevents_term_inflation() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_pre_vote: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader at term 1");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;
    n1.wait(timeout()).state(ServerState::Follower, "node 1 is follower").await?;

    let follower_term_before = n1.metrics().borrow_watched().current_term;
    let leader_term_before = n0.metrics().borrow_watched().current_term;

    tracing::info!("--- isolate node 1 and let many election timeouts pass");
    router.set_network_error(1, true);
    TypeConfig::sleep(Duration::from_secs(2)).await;

    tracing::info!("--- node 1's term must NOT have inflated");
    let follower_term_after = n1.metrics().borrow_watched().current_term;
    assert_eq!(
        follower_term_before, follower_term_after,
        "Pre-Vote must keep the isolated follower from bumping its term, was {}, now {}",
        follower_term_before, follower_term_after
    );

    tracing::info!("--- the leader kept leadership at the same term");
    n0.wait(timeout()).state(ServerState::Leader, "node 0 still leader").await?;
    assert_eq!(leader_term_before, n0.metrics().borrow_watched().current_term);

    tracing::info!("--- reconnect node 1; it rejoins as a follower without disruption");
    router.set_network_error(1, false);
    n1.wait(timeout()).state(ServerState::Follower, "node 1 rejoins as follower").await?;

    Ok(())
}

/// Baseline contrast: with Pre-Vote disabled (the default), the same isolated follower *does*
/// inflate its term. This proves the previous test actually exercises Pre-Vote rather than some
/// other quiescence.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn without_pre_vote_term_inflates() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_pre_vote: Some(false),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;
    n1.wait(timeout()).state(ServerState::Follower, "node 1 is follower").await?;

    let follower_term_before = n1.metrics().borrow_watched().current_term;

    tracing::info!("--- isolate node 1 and let many election timeouts pass");
    router.set_network_error(1, true);
    TypeConfig::sleep(Duration::from_secs(2)).await;

    let follower_term_after = n1.metrics().borrow_watched().current_term;
    assert!(
        follower_term_after > follower_term_before,
        "without Pre-Vote the isolated follower inflates its term, was {}, now {}",
        follower_term_before,
        follower_term_after
    );

    Ok(())
}

/// A manual `trigger().elect(true)` runs a Pre-Vote round first: against a healthy leader it is
/// declined, so the follower's term is left untouched and the leader is not disrupted.
///
/// This is the administrative-safety scenario: an operator triggering an election on a node that
/// cannot currently win must not inflate the cluster term. Pre-Vote is requested per call here — it
/// does not depend on `Config::enable_pre_vote`, which is left at its default.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn manual_elect_with_pre_vote_does_not_disrupt_leader() -> Result<()> {
    let config = Arc::new(Config::default().validate()?);

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;
    n1.wait(timeout()).state(ServerState::Follower, "node 1 is follower").await?;

    let follower_term_before = n1.metrics().borrow_watched().current_term;
    let leader_term_before = n0.metrics().borrow_watched().current_term;

    tracing::info!("--- node 1 manually triggers a cautious (pre-vote) election");
    n1.trigger().elect(true).await?;

    tracing::info!("--- give the Pre-Vote round time to be declined by the healthy peers");
    TypeConfig::sleep(Duration::from_millis(500)).await;

    tracing::info!("--- node 1's term is untouched and it stays a follower");
    assert_eq!(
        follower_term_before,
        n1.metrics().borrow_watched().current_term,
        "a declined Pre-Vote must not inflate the follower's term"
    );
    n1.wait(timeout()).state(ServerState::Follower, "node 1 stays follower").await?;

    tracing::info!("--- the leader kept leadership at the same term");
    n0.wait(timeout()).state(ServerState::Leader, "node 0 still leader").await?;
    assert_eq!(leader_term_before, n0.metrics().borrow_watched().current_term);

    Ok(())
}

/// Baseline contrast: a manual `trigger().elect(false)` starts a real election immediately and
/// inflates the follower's term, even against a healthy leader. This proves the previous test
/// exercises the Pre-Vote round rather than some other quiescence.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn manual_elect_without_pre_vote_inflates_term() -> Result<()> {
    let config = Arc::new(Config::default().validate()?);

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;
    n1.wait(timeout()).state(ServerState::Follower, "node 1 is follower").await?;

    let follower_term_before = n1.metrics().borrow_watched().current_term;

    tracing::info!("--- node 1 manually triggers a direct (no pre-vote) election");
    n1.trigger().elect(false).await?;

    tracing::info!("--- node 1's term inflates immediately");
    n1.wait(timeout())
        .metrics(
            |m| m.current_term > follower_term_before,
            "a direct manual election inflates the term",
        )
        .await?;

    Ok(())
}

/// With Pre-Vote enabled, a cluster must still elect a new leader after the current leader fails.
///
/// Pre-Vote makes elections *cautious*, not *impossible*: this is the core liveness requirement.
/// When the leader is isolated its lease expires on the remaining voters; they run a Pre-Vote
/// round, grant each other (no live leader holds a lease), and one wins and starts a real election.
/// If a node's in-flight Pre-Vote state were dropped before its peers' responses arrived, no voter
/// could ever assemble a Pre-Vote quorum and the cluster would stay leaderless forever.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn pre_vote_elects_new_leader_after_leader_isolated() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_pre_vote: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;

    tracing::info!("--- isolate the leader; nodes 1 and 2 still reach each other");
    router.set_network_error(0, true);

    tracing::info!("--- a new leader must emerge among 1,2 via the Pre-Vote path");
    for id in [1, 2] {
        router
            .wait(&id, Some(Duration::from_secs(5)))
            .metrics(
                |m| m.current_leader == Some(1) || m.current_leader == Some(2),
                "a new leader is elected via Pre-Vote after the old leader is isolated",
            )
            .await?;
    }

    Ok(())
}

/// A fully isolated voter whose peers are **unreachable** must not inflate its term, even when the
/// network implements `pre_vote`.
///
/// This guards the regression where an `Unreachable` Pre-Vote response was counted as a grant: an
/// isolated node would then synthesize a quorum of grants and run a real election anyway, defeating
/// Pre-Vote. The sibling [`pre_vote_prevents_term_inflation`] isolates with a `NetworkError`; this
/// one isolates with `Unreachable` specifically — the exact response a real network returns for a
/// partitioned peer, and the one previously (and wrongly) treated as a grant.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn pre_vote_unreachable_peer_is_not_a_grant() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_pre_vote: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader at term 1");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n1 = router.get_raft_handle(&1)?;
    n1.wait(timeout()).state(ServerState::Follower, "node 1 is follower").await?;

    let follower_term_before = n1.metrics().borrow_watched().current_term;

    tracing::info!("--- isolate node 1 with Unreachable and let many election timeouts pass");
    router.set_unreachable(1, true);
    TypeConfig::sleep(Duration::from_secs(2)).await;

    tracing::info!("--- node 1's term must NOT inflate: an Unreachable peer is not a grant");
    assert_eq!(
        follower_term_before,
        n1.metrics().borrow_watched().current_term,
        "an Unreachable Pre-Vote response must not count toward the quorum"
    );

    Ok(())
}

/// A rejected Pre-Vote response with a higher vote must catch the requester up to that vote.
///
/// This is the minimal public reproduction for databendlabs/openraft#1796: node 1 cannot reach a
/// quorum because node 0 is isolated, and node 2 rejects node 1's Pre-Vote with a higher vote. If
/// node 1 ignores that higher vote, it keeps retrying from the stale term forever.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn pre_vote_rejection_with_higher_vote_catches_up_requester() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            enable_pre_vote: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    tracing::info!("--- isolate node 0 so node 1 cannot assemble a Pre-Vote quorum");
    router.set_network_error(0, true);
    // Drain delayed AppendEntries heartbeats from node 0; one arriving during node 1's Pre-Vote
    // would reset `pre_candidate` and make node 1 ignore node 2's response.
    TypeConfig::sleep(Duration::from_millis(config.election_timeout_max * 2)).await;

    tracing::info!("--- move node 2 to a higher vote");
    let resp = n2
        .vote(VoteRequest {
            vote: Vote::new(10, 2),
            last_log_id: Some(log_id(1, 0, log_index)),
            leadership_transfer: true,
        })
        .await?;
    assert!(resp.vote_granted);
    router.wait(&2, timeout()).vote(Vote::new(10, 2), "node 2 has a higher vote").await?;

    tracing::info!("--- node 1 sees node 2's higher vote through a rejected Pre-Vote response");
    n1.trigger().elect(true).await?;
    router
        .wait(&1, Some(Duration::from_millis(1_000)))
        .vote(Vote::new(10, 2), "node 1 catches up from a higher Pre-Vote rejection")
        .await?;

    tracing::info!("--- node 1 retries and wins with node 2's vote");
    n1.trigger().elect(true).await?;
    n1.wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
