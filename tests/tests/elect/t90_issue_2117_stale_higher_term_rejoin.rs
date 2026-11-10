use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::Direction;
use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_error_type::RpcErrorType;
use crate::fixtures::ut_harness;

/// Long enough for several election timeouts at the default 150-300 ms.
const DISRUPTION_OBSERVATION: Duration = Duration::from_secs(2);

/// A rejoining node with a higher term and a shorter log does not stall the cluster.
///
/// `handle_vote_req` rejects such a node by `last_log_id` without adopting its term. The term still
/// reaches the quorum through replication: the leader's AppendEntries to the returned node comes
/// back as `HigherVote`, the leader steps down, and the cohort re-elects at a higher term. The
/// rejected node delays its next campaign by `smaller_log_timeout`, so the re-election wins the
/// race. Pre-Vote stays off so the isolated node's term really inflates. See issue #2117.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stale_higher_term_node_rejoins_without_pre_vote() -> Result<()> {
    let config = Config {
        enable_pre_vote: Some(false),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- establish node 0 as leader of a three-voter cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n1 = router.get_raft_handle(&1)?;
    let term_before = n1.metrics().borrow_watched().current_term;

    tracing::info!(
        log_index,
        term_before,
        "--- isolate node 1 both ways; commit writes it cannot see and let its term inflate"
    );
    let inflated_term = {
        router.set_network_error(1, true);
        router.client_request_many(0, "w", 10).await?;
        log_index += 10;
        router.wait_applied_at_least([0, 2], log_index, "writes applied on the quorum").await?;

        let inflated = router
            .wait(&1)
            .metrics(
                |m| m.current_term >= term_before + 3,
                "isolated node 1 inflates its term by at least 3",
            )
            .await?;
        assert!(
            inflated.last_log_index < Some(log_index),
            "node 1 missed the writes: last_log_index={:?} < {log_index}",
            inflated.last_log_index
        );
        inflated.current_term
    };

    tracing::info!(
        log_index,
        inflated_term,
        "--- reconnect node 1; it must follow a leader at a term not below its own and catch up"
    );
    {
        router.set_network_error(1, false);
        router.wait(&1).state(ServerState::Follower, "node 1 rejoins as follower").await?;
        router.wait_applied_at_least([0, 1, 2], log_index, "node 1 catches up").await?;

        let leaders: Vec<u64> = [0, 1, 2]
            .into_iter()
            .filter(|id| router.get_raft_handle(id).unwrap().metrics().borrow_watched().state == ServerState::Leader)
            .collect();
        assert_eq!(1, leaders.len(), "exactly one leader after the rejoin, got {leaders:?}");

        let leader_term = router.get_raft_handle(&leaders[0])?.metrics().borrow_watched().current_term;
        let n1_term = n1.metrics().borrow_watched().current_term;
        assert!(
            leader_term >= inflated_term,
            "the cohort moved past the inflated term: leader_term={leader_term} inflated_term={inflated_term}"
        );
        assert_eq!(leader_term, n1_term, "node 1 adopted the new leader's term");
    }

    Ok(())
}

/// A higher-term node that can send but not receive does not depose the leader.
///
/// Its vote requests arrive and are rejected by `last_log_id`; because the rejection does not
/// adopt the request's term, the quorum keeps its term and the leader keeps serving. Replication
/// toward the node fails, so no `HigherVote` comes back either. Adopting the term on rejection, as
/// issue #2117 proposed, would step the leader down for a node that can never win.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn one_way_isolated_higher_term_node_does_not_depose_leader() -> Result<()> {
    let config = Config {
        enable_pre_vote: Some(false),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- establish node 0 as leader of a three-voter cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    let leader_term = n0.metrics().borrow_watched().current_term;

    tracing::info!(
        log_index,
        leader_term,
        "--- block RPCs into node 1 only; commit writes it misses and let its term inflate"
    );
    let inflated_term = {
        router.set_rpc_failure(1, Direction::NetRecv, Some(RpcErrorType::NetworkError));
        router.client_request_many(0, "w", 10).await?;
        log_index += 10;
        router.wait_applied_at_least([0, 2], log_index, "writes applied on the quorum").await?;
        router
            .wait(&1)
            .metrics(
                |m| m.current_term >= leader_term + 3,
                "node 1 inflates its term by at least 3",
            )
            .await?
            .current_term
    };

    tracing::info!(
        log_index,
        inflated_term,
        "--- node 1's vote requests keep arriving; the leader must neither step down nor change term"
    );
    {
        TypeConfig::sleep(DISRUPTION_OBSERVATION).await;

        let n0_metrics = n0.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, n0_metrics.state, "node 0 keeps leading");
        assert_eq!(leader_term, n0_metrics.current_term, "the quorum's term is unchanged");
        assert!(
            n1.metrics().borrow_watched().current_term > inflated_term,
            "node 1 keeps campaigning at inflating terms"
        );

        router.client_request_many(0, "after", 3).await?;
        log_index += 3;
        router.wait_applied_at_least([0, 2], log_index, "the quorum still serves writes").await?;
    }

    Ok(())
}
