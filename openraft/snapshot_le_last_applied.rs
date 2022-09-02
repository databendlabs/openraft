use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::InstallSnapshotRequest;
use openraft::Config;
use openraft::LogId;
use openraft::RaftStorage;
use openraft::SnapshotMeta;

use crate::fixtures::RaftRouter;

/// Snapshot with smaller last_log_id than local last_applied should not be installed.
/// Otherwise state machine will revert to a older state.
#[tokio::test(flavor = "multi_thread", worker_threads = 10)]
async fn snapshot_le_last_applied() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(Config { ..Default::default() }.validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- send logs to increase last_applied");
    {
        router.client_request_many(0, "0", 10).await;
        log_index += 10;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;

        router.assert_stable_cluster(Some(1), Some(log_index)).await;
    }

    tracing::info!("--- it should fail to install a snapshot with smaller last_log_id");
    {
        assert!(log_index > 3);
        let req = InstallSnapshotRequest {
            term: 100,
            leader_id: 1,
            meta: SnapshotMeta {
                snapshot_id: "ss1".into(),
                last_log_id: Some(LogId { term: 1, index: 3 }),
            },
            offset: 0,
            data: vec![1, 2, 3],
            done: true,
        };

        let n = router.remove_node(0).await.unwrap();
        n.0.install_snapshot(req).await?;
        let st = n.1.last_applied_state().await?;
        assert_eq!(
            Some(LogId {
                term: 1,
                index: log_index
            }),
            st.0,
            "last_applied is not affected"
        );

        let wait_rs =
            n.0.wait(timeout())
                .metrics(
                    |x| {
                        x.last_applied
                            != Some(LogId {
                                term: 1,
                                index: log_index,
                            })
                    },
                    "in-memory last_applied is not affected",
                )
                .await;

        assert!(wait_rs.is_err());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
