use openraft::Config;
use openraft::Vote;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn install_full_snapshot() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- isolate node 2 so that it can receive snapshot");
    router.set_unreachable(2, true);

    tracing::info!(log_index, "--- write to make node-0,1 have more logs");
    {
        let written = router.client_request_many(0, "foo", 3).await?;
        let last_log_id = written.last_log_id.expect("nonempty workload");
        log_index = last_log_id.index();
        router.wait_applied_at_least([0, 1], log_index, "write more log").await?;
    }

    let snap;

    tracing::info!(log_index, "--- trigger and get snapshot from node-0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        router.wait(&0).snapshot(log_id(1, 0, log_index), "node-1 snapshot").await?;

        snap = n0.get_snapshot().await?.unwrap();
    }

    tracing::info!(log_index, "--- fails to install snapshot with smaller vote");
    {
        let n1 = router.get_raft_handle(&1)?;

        let resp = n1.install_full_snapshot(Vote::new(0, 0), snap.clone()).await?;
        assert_eq!(
            Vote::new_committed(1, 0),
            resp.vote,
            "node-1 vote is higher, and is returned"
        );
        n1.with_raft_state(|state| {
            assert_eq!(
                None, state.snapshot_meta.last_log_id,
                "node-1 snapshot is not installed"
            );
        })
        .await?;
    }

    tracing::info!(
        log_index,
        "--- no install snapshot on node-1 because of snapshot last log id equals committed"
    );
    {
        let n1 = router.get_raft_handle(&1)?;

        let resp = n1.install_full_snapshot(Vote::new_committed(1, 0), snap.clone()).await?;
        assert_eq!(Vote::new_committed(1, 0), resp.vote,);
        n1.with_raft_state(move |state| {
            assert_eq!(
                None, state.snapshot_meta.last_log_id,
                "node-1 snapshot is not installed"
            );
        })
        .await?;
    }

    tracing::info!(log_index, "--- succeed to install snapshot on node-2 with higher vote");
    {
        let n2 = router.get_raft_handle(&2)?;

        let resp = n2.install_full_snapshot(Vote::new_committed(1, 0), snap.clone()).await?;
        assert_eq!(Vote::new_committed(1, 0), resp.vote,);
        n2.with_raft_state(move |state| {
            assert_eq!(
                Some(log_id(1, 0, log_index)),
                state.snapshot_meta.last_log_id,
                "node-1 snapshot is installed"
            );
        })
        .await?;
    }

    Ok(())
}
