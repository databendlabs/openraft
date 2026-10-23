use maplit::hashmap;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::impls::Vote;
use openraft::testing::blank_ent;
use openraft_memstore::TypeConfig;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A single leader should re-apply all logs upon startup,
/// because itself is a quorum.
///
/// This test disables save_committed() to ensure that logs are still re-applied because the leader
/// itself forms a quorum.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn single_leader_restart_re_apply_logs() -> anyhow::Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    router.enable_saving_committed = false;

    tracing::info!("--- bring up cluster of 1 node");
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- write to 1 log");
    {
        let response = router.client_request(0, "foo", 0).await?;
        log_index = response.log_id.index();
    }

    tracing::info!(log_index, "--- stop and restart node-0");
    {
        let Node {
            log_store: ls,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;

        // Clear state machine, logs should be re-applied upon restart, because it is a leader.
        sm.clear_state_machine().await;

        tracing::info!(log_index, "--- restart node-0");

        router.new_raft_node_with_sto(0, ls, sm).await?;
        router.wait(&0).vote(Vote::new_committed(2, 0), "startup election succeeds without ticks").await?;
        router.wait(&0).state(ServerState::Leader, "leader after startup election").await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- a single leader should re-apply all logs");
    {
        router.wait(&0).applied_index(Some(log_index), "logs re-applied, plus the term-2 blank").await?;

        let (mut ls, sm) = router.get_storage_handle(&0)?;
        let entries = ls.try_get_log_entries(log_index..).await?;
        let want = vec![blank_ent::<TypeConfig>(2, 0, log_index)];
        let entries = bincode::serialize(&entries)?;
        let want = bincode::serialize(&want)?;
        assert_eq!(want, entries);

        let sm = sm.get_state_machine().await;
        let want = hashmap! {"foo".to_string() => "request-0".to_string()};
        assert_eq!(want, sm.client_status);
    }

    Ok(())
}
