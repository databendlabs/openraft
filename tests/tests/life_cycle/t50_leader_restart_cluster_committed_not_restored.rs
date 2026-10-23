use openraft::Config;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::RaftError;
use openraft::impls::Vote;
use openraft_memstore::TypeConfig;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A former leader without a quorum stays a candidate and restores only its local commit.
///
/// `cluster_committed` is established only by replicating to a quorum; it is never restored from
/// storage. The startup campaign must win before the node can propose entries, so a rejected
/// client write must leave the complete log unchanged.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_restart_cluster_committed_not_restored() -> anyhow::Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a 3-node cluster; node-0 is the leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let committed = Some(log_id(1, 0, log_index));

    tracing::info!(log_index, "--- the live leader has an established cluster commit");
    {
        let m = router.get_metrics(&0)?;
        assert_eq!(committed, m.cluster_committed);
        assert_eq!(committed, m.local_committed);
    }

    tracing::info!(
        log_index,
        "--- take both followers offline so the leader loses its quorum"
    );
    for id in [1, 2] {
        router.stop_node(id).await?;
    }

    tracing::info!(
        log_index,
        "--- restart node-0 with its persisted state; it campaigns in a higher term"
    );
    {
        let Node {
            log_store: ls,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;

        router.new_raft_node_with_sto(0, ls, sm).await?;
        router.wait(&0).vote(Vote::new(2, 0), "startup campaign advances the term").await?;
        router.wait(&0).state(ServerState::Candidate, "no quorum to establish leadership").await?;
        router.wait(&0).applied_index(Some(log_index), "applied restored from storage").await?;
    }

    tracing::info!(
        log_index,
        "--- local_committed is restored from storage, cluster_committed is not"
    );
    {
        let m = router.get_metrics(&0)?;

        assert_eq!(committed, m.local_committed);
        assert_eq!(committed, m.last_applied);
        assert_eq!(None, m.cluster_committed);
        assert_eq!(None, m.current_leader);
    }

    tracing::info!(
        log_index,
        "--- a client write must be rejected without creating a log entry"
    );
    {
        let (mut ls, _) = router.get_storage_handle(&0)?;
        let entries = ls.try_get_log_entries(..).await?;
        let before = bincode::serialize(&entries)?;
        let result = router.client_request(0, "rejected", 0).await;
        let error = result.expect_err("a candidate must reject client writes");
        let forward = ForwardToLeader::<TypeConfig>::empty();
        let write_error = ClientWriteError::ForwardToLeader(forward);
        let want = RaftError::APIError(write_error);
        assert_eq!(want, error);

        let entries = ls.try_get_log_entries(..).await?;
        let after = bincode::serialize(&entries)?;
        assert_eq!(before, after);
        let m = router.get_metrics(&0)?;
        assert_eq!(ServerState::Candidate, m.state);
        assert_eq!(Some(log_index), m.last_log_index);
        assert_eq!(committed, m.local_committed);
        assert_eq!(None, m.cluster_committed);
    }

    Ok(())
}
