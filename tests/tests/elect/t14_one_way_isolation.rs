use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Instant;
use openraft::ServerState;
use openraft::TokioInstant;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::Direction;
use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_error_type::RpcErrorType;
use crate::fixtures::ut_harness;

const ELECTION_TIMEOUT_MIN: u64 = 150;
const ELECTION_TIMEOUT_MAX: u64 = 151;
const ELECTION_OBSERVATION: Duration = Duration::from_millis(ELECTION_TIMEOUT_MAX * 2);
const WAIT_TIMEOUT: Duration = Duration::from_millis(2_000);

/// A follower must not campaign while it can still receive heartbeats, even if all of its
/// outbound RPCs fail.
///
/// `NetSend` on node 1 leaves `AppendEntries` from node 0 deliverable. The test first observes a
/// heartbeat received after installing the fault, then waits through two maximum election
/// timeouts. Pre-Vote is disabled, so any election attempt would change node 1's vote; retaining
/// the same committed vote and follower state proves that inbound heartbeats keep renewing its
/// leader lease.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn inbound_heartbeats_prevent_election_when_outbound_rpcs_fail() -> Result<()> {
    let config = Config {
        enable_pre_vote: Some(false),
        election_timeout_min: ELECTION_TIMEOUT_MIN,
        election_timeout_max: ELECTION_TIMEOUT_MAX,
        ..Default::default()
    }
    .validate()?;
    let mut router = RaftRouter::new(Arc::new(config));

    tracing::info!("--- establish node 0 as leader and node 1 as follower in a three-voter cluster");
    let log_index = { router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await? };

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    tracing::info!(
        log_index,
        "--- wait until node 1 recognizes node 0 as leader before installing the fault"
    );
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
        vote = %before.vote,
        "--- block node 1 outbound RPCs and prove it can still receive heartbeats"
    );
    {
        router.set_rpc_failure(1, Direction::NetSend, Some(RpcErrorType::NetworkError));

        let heartbeat_at = TokioInstant::now();
        n0.trigger().heartbeat().await?;

        let mut heartbeat_received = false;
        for _ in 0..20 {
            let last_modified = router.with_raft_state(1, |state| state.vote_last_modified()).await?;
            if last_modified > Some(heartbeat_at) {
                heartbeat_received = true;
                break;
            }

            TypeConfig::sleep(Duration::from_millis(50)).await;
        }
        assert!(heartbeat_received, "node 1 must receive the forced heartbeat");
    }

    tracing::info!(
        log_index,
        "--- let two election timeouts pass while inbound heartbeats renew node 1's lease"
    );
    {
        TypeConfig::sleep(ELECTION_OBSERVATION).await;
    }

    tracing::info!(
        log_index,
        "--- node 1 keeps the same vote and remains a follower of node 0"
    );
    {
        let after = n1.metrics().borrow_watched().clone();
        assert_eq!(
            before.current_term, after.current_term,
            "node 1 must not advance its term"
        );
        assert_eq!(before.vote, after.vote, "node 1 must retain its vote");
        assert_eq!(
            Some(0),
            after.current_leader,
            "node 1 must still recognize node 0 as leader"
        );
        assert_eq!(ServerState::Follower, after.state, "node 1 must remain a follower");
    }

    Ok(())
}

/// A follower must campaign when it cannot receive heartbeats but can still send outbound RPCs.
///
/// `NetRecv` on node 1 blocks `AppendEntries` from node 0 without blocking node 1's own Vote RPCs.
/// With Pre-Vote disabled, the first expired leader lease starts a real election: node 1 advances
/// its term and votes for itself.
///
/// The test asserts the term and the vote, not the `Candidate` state. Node 0 never renews the
/// lease on its own vote, so node 0 grants the vote request at once and node 1 becomes Leader
/// about a millisecond after campaigning. `Candidate` is transient and the metrics watch channel
/// may skip it. The higher term and the self-vote persist, because every RPC that could move node
/// 1's vote is blocked.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn missing_inbound_heartbeats_start_election() -> Result<()> {
    let config = Config {
        enable_pre_vote: Some(false),
        election_timeout_min: ELECTION_TIMEOUT_MIN,
        election_timeout_max: ELECTION_TIMEOUT_MAX,
        ..Default::default()
    }
    .validate()?;
    let mut router = RaftRouter::new(Arc::new(config));

    tracing::info!("--- establish node 0 as leader and node 1 as follower in a three-voter cluster");
    let log_index = { router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await? };

    let n1 = router.get_raft_handle(&1)?;
    tracing::info!(
        log_index,
        "--- wait until node 1 recognizes node 0 as leader before installing the fault"
    );
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
        vote = %before.vote,
        "--- block node 1 inbound RPCs while keeping its outbound Vote RPCs available"
    );
    {
        router.set_rpc_failure(1, Direction::NetRecv, Some(RpcErrorType::NetworkError));
    }

    tracing::info!(log_index, "--- node 1 misses heartbeats and starts a direct election");
    {
        let campaigned = router
            .wait(&1, Some(WAIT_TIMEOUT))
            .metrics(
                |metrics| metrics.current_term > before.current_term,
                "node 1 advances its term after its leader lease expires",
            )
            .await?;

        let expected_vote = VoteOf::<TypeConfig>::new(campaigned.current_term, 1);
        assert_eq!(
            expected_vote.leader_id(),
            campaigned.vote.leader_id(),
            "node 1 must install its self-vote"
        );
    }

    Ok(())
}
