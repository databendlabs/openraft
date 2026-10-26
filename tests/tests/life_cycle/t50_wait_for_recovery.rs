use std::time::Duration;

use openraft::Config;
use openraft::ServerState;
use openraft::metrics::WaitError;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// `wait_for_recovery` returns once the cluster commit is re-established and the state machine has
/// applied up to it.
///
/// A follower restarts with a cleared (transient) state machine and `save_committed` disabled, so
/// nothing is recovered locally on startup — recovery is driven entirely by the cluster commit the
/// leader replicates. The surviving `{0,2}` quorum keeps that commit established.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn wait_for_recovery_recovers_state_machine() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: true,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    // Recovery must come from the cluster commit, not a locally restored committed pointer.
    router.enable_saving_committed = false;

    tracing::info!("--- bring up a 3-node cluster; node-0 is the leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- restart follower node-1 with a cleared state machine");
    {
        let Node {
            log_store: ls,
            state_machine: sm,
            ..
        } = router.stop_node(1).await?;

        // Transient state machine: in-memory applied state is lost on restart.
        sm.clear_state_machine().await;

        router.new_raft_node_with_sto(1, ls, sm).await?;
    }

    tracing::info!(
        log_index,
        "--- wait_for_recovery returns once the state machine catches up"
    );
    {
        let m = router.get_raft_handle(&1)?.wait_for_recovery(timeout()).await?;

        assert!(m.cluster_committed.is_some(), "the cluster commit is perceived");
        assert!(
            m.last_applied.as_ref().map(|x| x.index()) >= Some(log_index),
            "state machine recovered: last_applied {:?} >= {}",
            m.last_applied,
            log_index
        );
    }

    Ok(())
}

/// `wait_for_recovery` times out when no cluster commit can be re-established.
///
/// A restarted leader that has lost its quorum never perceives a non-null cluster commit, so the
/// recovery wait cannot complete and returns [`WaitError::Timeout`].
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn wait_for_recovery_times_out_without_quorum() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a 3-node cluster; node-0 is the leader");
    let _log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!("--- take both followers offline so the leader loses its quorum");
    for id in [1, 2] {
        router.stop_node(id).await?;
    }

    tracing::info!("--- restart node-0; it restores leadership but cannot re-establish the commit");
    {
        let Node {
            log_store: ls,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;

        router.new_raft_node_with_sto(0, ls, sm).await?;
        router.wait(&0).state(ServerState::Leader, "restore leadership without election").await?;
    }

    tracing::info!("--- with no quorum the cluster commit stays null, so recovery times out");
    {
        let res = router.get_raft_handle(&0)?.wait_for_recovery(Some(Duration::from_millis(500))).await;
        assert!(
            matches!(res, Err(WaitError::Timeout(..))),
            "expected a timeout, got {:?}",
            res
        );
    }

    Ok(())
}
