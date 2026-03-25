use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::base::BoxFuture;
use openraft::Config;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Bug #5: establish_leader() update_vote failure silently swallowed by debug_assert.
///
/// This test reproduces a bug where `establish_leader()` calls `update_vote()` with the
/// candidate's committed vote, but the return value is only checked by `debug_assert!`.
/// When `update_vote` fails (because `state.vote` was bumped to a higher term by a
/// rejected vote response), the failure is silently swallowed in release builds.
///
/// Scenario (state reversion — within openraft's design scope):
/// 1. 3-node cluster. Node 1 becomes leader so node 0 is a follower (no leader struct).
/// 2. We inject Vote(T7, 0, committed) into node 1 via a fabricated heartbeat,
///    simulating state from before a reversion.
/// 3. Node 0 starts election (at some term < T7).
/// 4. Node 1 rejects (has Vote(T7, 0, committed)). The rejection is processed first
///    via timing control. handle_vote_resp applies to_non_committed() -> Vote(T7, 0, uncommitted).
///    state.vote is bumped to T7. Candidate survives (same node_id).
/// 5. Node 2 grants -> quorum -> establish_leader() calls update_vote(committed at low term).
///    But state.vote = Vote(T7, uncommitted) > committed at low term -> update_vote FAILS.
///    debug_assert! panics in debug; silently swallowed in release.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn bug5_update_vote_debug_assert() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            // election_timeout_min is used as the TTL for vote RPCs.
            // It needs to be large enough for our timing control (~500ms gate + 100ms delay).
            // But the leader lease is based on election_timeout_max, so keep it reasonable.
            heartbeat_interval: 100,
            election_timeout_min: 5_000,
            election_timeout_max: 5_001,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- create 3-node cluster, node 0 is initial leader at term 1");
    let log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 is leader").await?;

    tracing::info!(log_index, "--- let leadership expire, then have node 1 become leader");
    // Wait for the leader lease to expire on all nodes (election_timeout_max = 5001ms).
    // This makes node 0 step down to Follower/Learner, clearing its leader struct.
    // This is critical: we need node 0 to NOT have an active leader struct when it
    // starts its election, otherwise a different debug_assert fires first.
    TypeConfig::sleep(Duration::from_millis(6000)).await;

    let n1 = router.get_raft_handle(&1)?;
    n1.trigger().elect().await?;
    n1.wait(timeout()).state(ServerState::Leader, "node 1 is leader").await?;

    // Node 0 should have stepped down
    n0.wait(timeout())
        .state(ServerState::Follower, "node 0 stepped down to follower")
        .await?;

    let m0 = router.get_metrics(&0)?;
    let m1 = router.get_metrics(&1)?;
    tracing::info!("node 0: term={}, vote={}", m0.current_term, m0.vote);
    tracing::info!("node 1: term={}, vote={}, state={:?}", m1.current_term, m1.vote, m1.state);
    let current_term = m0.current_term;

    tracing::info!(log_index, "--- inject Vote(T7, 0, committed) into node 1");
    // Send a fabricated AppendEntries heartbeat from "node 0 at term 7".
    // This causes node 1 to accept Vote(T7, 0, committed), overriding its current vote.
    // This simulates node 1 having previously seen node 0 as leader at a very high term.
    {
        let high_term_vote = Vote::new_committed(7, 0);
        // Use the current last_log_id for prev_log_id
        let m1 = router.get_metrics(&1)?;
        let last_log_idx = m1.last_log_index.unwrap_or(log_index);
        let last_log = log_id(current_term, 1, last_log_idx);

        let req = AppendEntriesRequest::<TypeConfig> {
            vote: high_term_vote.clone(),
            prev_log_id: Some(last_log.clone()),
            entries: vec![],
            leader_commit: Some(last_log),
        };

        let resp = n1.append_entries(req).await?;
        tracing::info!("inject response: {:?}", resp);

        n1.wait(timeout())
            .vote(high_term_vote, "node 1 accepts Vote(T7, 0, committed)")
            .await?;

        let m1 = router.get_metrics(&1)?;
        tracing::info!("node 1 after inject: vote={}, term={}", m1.vote, m1.current_term);
        assert_eq!(m1.current_term, 7, "node 1 should be at term 7");
    }

    tracing::info!(log_index, "--- set up Vote pre-hook for timing control");
    let gate_open = Arc::new(AtomicBool::new(false));
    let gate_for_pre = gate_open.clone();

    router
        .set_rpc_pre_hook(RPCTypes::Vote, move |_router, _req, _from, target| {
            let gate = gate_for_pre.clone();
            let fu: BoxFuture<_> = if target == 2 {
                Box::pin(async move {
                    for _ in 0..500 {
                        if gate.load(Ordering::Acquire) {
                            TypeConfig::sleep(Duration::from_millis(100)).await;
                            return Ok(());
                        }
                        TypeConfig::sleep(Duration::from_millis(10)).await;
                    }
                    tracing::warn!("gate timed out");
                    Ok(())
                })
            } else {
                Box::pin(futures::future::ready(Ok(())))
            };
            fu
        })
        .await;

    let gate_for_post = gate_open.clone();
    router
        .set_rpc_post_hook(RPCTypes::Vote, move |_router, _req, _resp, _from, target| {
            if target == 1 {
                tracing::info!("post-hook: node 1 responded, opening gate for node 2");
                gate_for_post.store(true, Ordering::Release);
            }
            let fu: BoxFuture<_> = Box::pin(futures::future::ready(Ok(())));
            fu
        })
        .await;

    tracing::info!(log_index, "--- wait for all leases to expire before election");
    // Node 2 holds a committed vote for node 1 (with leader lease from T2).
    // The lease duration is election_timeout_max = 5001ms.
    // We must wait for this lease to expire so node 2 will accept node 0's VoteRequest.
    TypeConfig::sleep(Duration::from_millis(6000)).await;

    tracing::info!(log_index, "--- trigger election on node 0");
    let m0_before = router.get_metrics(&0)?;
    let election_term = m0_before.current_term + 1;
    tracing::info!(
        "node 0 at term {}, will elect at term {}",
        m0_before.current_term,
        election_term
    );

    // Node 0 triggers election at election_term (current_term + 1).
    // election_term < 7, so:
    //   - Node 1 (at T7) rejects -> to_non_committed -> Vote(T7, 0, uncommitted) bumps state.vote
    //   - Node 2 grants -> quorum -> establish_leader -> update_vote(election_term, committed) FAILS
    n0.trigger().elect().await?;

    TypeConfig::sleep(Duration::from_millis(1500)).await;

    router.rpc_pre_hook(RPCTypes::Vote, None).await;
    router.rpc_post_hook(RPCTypes::Vote, None).await;

    tracing::info!("--- verify the bug");
    let m0 = router.get_metrics(&0)?;
    tracing::info!(
        "node 0 after election: running_state={:?}, state={:?}, vote={}, term={}",
        m0.running_state,
        m0.state,
        m0.vote,
        m0.current_term,
    );

    // The bug manifests as:
    // - Debug builds: debug_assert panic kills the Raft core -> fatal or term >= 7
    // - Release builds: state.vote = Vote(T7, 0, uncommitted) silently diverged
    let bug_triggered = m0.current_term >= 7 || m0.running_state.is_err();

    if bug_triggered {
        tracing::info!("CONFIRMED: Bug #5 reproduced.");
        if m0.running_state.is_err() {
            tracing::info!(
                "  Debug build: Raft core panicked from debug_assert: {:?}",
                m0.running_state
            );
        } else {
            tracing::info!(
                "  Release build: state.vote = {} (committed={}). Expected Vote(T{}, committed=true).",
                m0.vote,
                m0.vote.is_committed(),
                election_term,
            );
        }
    }

    assert!(
        bug_triggered,
        "Bug #5 should be triggered: expected term >= 7 or fatal state, \
         got term={}, running_state={:?}",
        m0.current_term,
        m0.running_state,
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
