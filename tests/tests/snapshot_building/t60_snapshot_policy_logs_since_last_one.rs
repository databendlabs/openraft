use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::SnapshotPolicy;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// With `LogsSinceLast(1)`, an automatic snapshot attempt that completes or is refused is not
/// repeated at the same committed position, and a newly committed log permits the next attempt.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_policy_logs_since_last_one() -> Result<()> {
    let config = Config {
        snapshot_policy: SnapshotPolicy::LogsSinceLast(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a single node: its committed logs are snapshotted");
    let mut log_index = router.new_cluster([0], []).await?;
    router.wait(&0).snapshot(log_id(1, 0, log_index), "snapshot at initialization").await?;
    let (_, sm) = router.get_storage_handle(&0)?;
    let attempts = || sm.try_create_snapshot_builder_count.load(Ordering::Relaxed);

    tracing::info!(log_index, "--- a completed build is not repeated without a new commit");
    {
        let before = attempts();
        TypeConfig::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            before,
            attempts(),
            "a completed build was repeated at the same position"
        );
    }

    tracing::info!(log_index, "--- a refused build is not repeated without a new commit");
    {
        sm.allow_build_snapshot(false);
        let before = attempts();
        router.client_request(0, "foo", 1).await?;
        log_index += 1;
        router.wait(&0).applied_index(Some(log_index), "write one log").await?;

        TypeConfig::timeout(Duration::from_secs(5), async {
            while attempts() == before {
                TypeConfig::sleep(Duration::from_millis(1)).await;
            }
        })
        .await?;
        let refused = attempts();
        assert_eq!(before + 1, refused, "the new commit permits exactly one attempt");

        TypeConfig::sleep(Duration::from_millis(500)).await;
        assert_eq!(refused, attempts(), "a refused build was repeated at the same position");
    }

    tracing::info!(log_index, "--- the next commit permits a build that completes");
    {
        sm.allow_build_snapshot(true);
        router.client_request(0, "foo", 2).await?;
        log_index += 1;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "snapshot after a new commit").await?;
    }

    Ok(())
}
