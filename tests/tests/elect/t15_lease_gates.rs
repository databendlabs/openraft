use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Instant;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::TokioInstant;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

const ELECTION_TIMEOUT_MIN: u64 = 150;
const ELECTION_TIMEOUT_MAX: u64 = 151;
/// `leader_lease` equals `election_timeout_max`, see `EngineConfig::new`.
const LEADER_LEASE: Duration = Duration::from_millis(ELECTION_TIMEOUT_MAX);
const WAIT_TIMEOUT: Duration = Duration::from_millis(5_000);

fn config() -> Result<Arc<Config>> {
    let config = Config {
        election_timeout_min: ELECTION_TIMEOUT_MIN,
        election_timeout_max: ELECTION_TIMEOUT_MAX,
        ..Default::default()
    }
    .validate()?;
    Ok(Arc::new(config))
}

/// Block one direction of one link for AppendEntries only, so heartbeats stop flowing on
/// that link while every Vote and Pre-Vote RPC stays deliverable.
async fn block_append_entries(router: &RaftRouter, from: u64, to: u64) {
    router
        .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, _req, f, t| {
            let res = if f == from && t == to {
                let err = NetworkError::<TypeConfig>::from_string(format!("blocked: {}->{} append-entries", f, t));
                Err(RPCError::Network(err))
            } else {
                Ok(())
            };
            Box::pin(futures::future::ready(res))
        })
        .await;
}

/// Wait until `node`'s local `state.vote` lease has expired: its `vote_last_modified` stops
/// advancing once heartbeats are blocked, so poll until `now` has passed it by the lease.
async fn wait_vote_lease_expired(router: &RaftRouter, node: u64) -> Result<()> {
    for _ in 0..200 {
        let last_modified = router.with_raft_state(node, |state| state.vote_last_modified()).await?;
        let now = TokioInstant::now();

        let expired = match last_modified {
            Some(last_modified) => now > last_modified + LEADER_LEASE,
            None => true,
        };
        if expired {
            return Ok(());
        }

        TypeConfig::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("node {} vote lease did not expire in time", node);
}

/// A manual Pre-Vote election from a follower whose own leader lease has expired unseats a
/// leader that a quorum keeps acking.
///
/// Topology: leader node 0 misses no acks — node 2 keeps acking it, and `{0, 2}` is a
/// quorum — but the 0→1 link drops AppendEntries, so node 1's own leader lease expires.
/// Node 1 then runs a manual cautious election: `trigger().elect(true)`.
///
/// This reproduces `leader-self-vote-lease-not-renewed.md`: node 0 never renews the lease
/// on its own `state.vote`, so it grants node 1's Pre-Vote and vote although its quorum-ack
/// lease (`Leader::last_quorum_acked_time`) is fresh. Node 1 plus node 0 is a quorum, and
/// node 1 seizes leadership.
///
/// The fix rejects votes and Pre-Votes while the leader's quorum-ack lease is valid; the
/// fix commit flips the assertions to: every term unchanged and node 0 stays leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn manual_pre_vote_from_lease_expired_follower() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    tracing::info!("--- establish node 0 as leader of a three-voter cluster");
    let log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;

    let before = {
        n1.wait(Some(WAIT_TIMEOUT))
            .metrics(
                |metrics| metrics.state == ServerState::Follower && metrics.current_leader == Some(0),
                "node 1 follows node 0",
            )
            .await?
    };

    tracing::info!(
        log_index,
        term = before.current_term,
        "--- disable node 1's automatic election; the manual trigger is unaffected"
    );
    {
        n1.runtime_config().elect(false);
    }

    tracing::info!(log_index, "--- drop 0->1 AppendEntries and wait for node 1's lease to expire");
    {
        block_append_entries(&router, 0, 1).await;
        wait_vote_lease_expired(&router, 1).await?;
    }

    tracing::info!(log_index, "--- refresh node 0's quorum-ack lease through node 2");
    {
        let heartbeat_at = TokioInstant::now();
        n0.trigger().heartbeat().await?;
        n0.wait(Some(WAIT_TIMEOUT))
            .metrics(
                |metrics| {
                    metrics
                        .last_quorum_acked
                        .is_some_and(|acked| acked.into_inner() >= heartbeat_at)
                },
                "node 0's quorum-ack lease is fresh after the forced heartbeat",
            )
            .await?;
    }

    tracing::info!(log_index, "--- node 1 manually triggers a cautious (pre-vote) election");
    {
        n1.trigger().elect(true).await?;
    }

    tracing::info!(log_index, "--- node 0 grants although a quorum keeps acking it; node 1 seizes leadership");
    {
        n1.wait(Some(WAIT_TIMEOUT))
            .state(ServerState::Leader, "node 1 becomes leader through node 0's stale vote lease")
            .await?;

        let after = n1.metrics().borrow_watched().clone();
        assert_eq!(
            before.current_term + 1,
            after.current_term,
            "the manual election bumped the term"
        );
    }

    Ok(())
}

/// A manual Pre-Vote election from a healthy follower unseats the leader when one other
/// follower's leader lease has expired.
///
/// Topology: leader node 0 is acked by node 1, so `{0, 1}` keeps node 0's quorum-ack lease
/// fresh, and node 1 also keeps receiving heartbeats. The 0→2 link drops AppendEntries, so
/// node 2's leader lease expires. Node 1 — the healthy follower — runs a manual cautious
/// election: `trigger().elect(true)`.
///
/// This reproduces the second gap in `leader-self-vote-lease-not-renewed.md`: `pre_elect`
/// grants the Pre-Vote to itself without consulting node 1's own fresh lease, and node 2's
/// lease has expired, so `{1, 2}` is a quorum with no healthy grantor needed. Today node 0
/// also grants through its stale vote lease.
///
/// The fix makes `pre_elect` refuse while the local lease is valid, so the round never
/// starts; the fix commit flips the assertions to: every term unchanged and node 0 stays
/// leader. Blocking only 0→2 AppendEntries — with 1→2 Pre-Vote deliverable — keeps this
/// test able to fail without the local gate: node 2 would grant.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn manual_pre_vote_from_healthy_follower_with_stale_peer() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    tracing::info!("--- establish node 0 as leader of a three-voter cluster");
    let log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    let before = {
        n1.wait(Some(WAIT_TIMEOUT))
            .metrics(
                |metrics| metrics.state == ServerState::Follower && metrics.current_leader == Some(0),
                "node 1 follows node 0",
            )
            .await?
    };

    tracing::info!(
        log_index,
        term = before.current_term,
        "--- disable node 2's automatic election so its expired lease cannot start a race"
    );
    {
        n2.runtime_config().elect(false);
    }

    tracing::info!(log_index, "--- drop 0->2 AppendEntries and wait for node 2's lease to expire");
    {
        block_append_entries(&router, 0, 2).await;
        wait_vote_lease_expired(&router, 2).await?;
    }

    tracing::info!(log_index, "--- refresh node 0's quorum-ack lease and node 1's follower lease");
    {
        let heartbeat_at = TokioInstant::now();
        n0.trigger().heartbeat().await?;
        n0.wait(Some(WAIT_TIMEOUT))
            .metrics(
                |metrics| {
                    metrics
                        .last_quorum_acked
                        .is_some_and(|acked| acked.into_inner() >= heartbeat_at)
                },
                "node 0's quorum-ack lease is fresh after the forced heartbeat",
            )
            .await?;
    }

    tracing::info!(log_index, "--- healthy node 1 manually triggers a cautious (pre-vote) election");
    {
        n1.trigger().elect(true).await?;
    }

    tracing::info!(log_index, "--- the quorum of 1 and 2 elects node 1 although node 1 still hears node 0");
    {
        n1.wait(Some(WAIT_TIMEOUT))
            .state(ServerState::Leader, "node 1 becomes leader with the stale peer's grant")
            .await?;

        let after = n1.metrics().borrow_watched().clone();
        assert_eq!(
            before.current_term + 1,
            after.current_term,
            "the manual election bumped the term"
        );
    }

    Ok(())
}
