use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::SnapshotPolicy;
use openraft::StorageHelper;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test a second compaction should not lose membership.
/// To ensure the bug is fixed:
/// - Snapshot stores membership when compaction.
/// - But compaction does not extract membership config from Snapshot entry, only from
///   MembershipConfig entry.
///
/// What does this test do?
///
/// - build a cluster of 2 nodes.
/// - send enough requests to trigger a snapshot.
/// - send just enough request to trigger another snapshot.
/// - ensure membership is still valid.

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_uses_prev_snap_membership() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            // Use 3, with 1 it triggers a compaction when replicating ent-1,
            // because ent-0 is removed.
            max_in_snapshot_log_to_keep: 3,
            purge_batch_size: 1,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    let (mut sto0, mut sm0) = router.get_storage_handle(&0)?;

    tracing::info!(log_index, "--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

        for id in [0, 1] {
            router.wait(&id, timeout()).applied_index(Some(log_index), "send log to trigger snapshot").await?;
        }
        router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "1st snapshot").await?;

        {
            let logs = sto0.try_get_log_entries(..).await?;
            assert_eq!(3, logs.len(), "only one applied log is kept");
        }
        let m = StorageHelper::new(&mut sto0, &mut sm0).get_membership().await?;

        assert_eq!(
            &Membership::new_with_defaults(vec![btreeset! {0,1}], []),
            m.committed().membership(),
            "membership "
        );
        assert_eq!(
            &Membership::new_with_defaults(vec![btreeset! {0,1}], []),
            m.effective().membership(),
            "membership "
        );
    }

    tracing::info!(log_index, "--- send just enough logs to trigger the 2nd snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold * 2 - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold * 2 - 1;

        for id in [0, 1] {
            router.wait(&id, None).applied_index(Some(log_index), "send log to trigger snapshot").await?;
        }
        router.wait(&0, None).snapshot(log_id(1, 0, log_index), "2nd snapshot").await?;
    }

    tracing::info!(log_index, "--- check membership");
    {
        {
            let logs = sto0.try_get_log_entries(..).await?;
            assert_eq!(3, logs.len(), "only one applied log");
        }
        let m = StorageHelper::new(&mut sto0, &mut sm0).get_membership().await?;

        assert_eq!(
            &Membership::new_with_defaults(vec![btreeset! {0,1}], []),
            m.committed().membership(),
            "membership "
        );
        assert_eq!(
            &Membership::new_with_defaults(vec![btreeset! {0,1}], []),
            m.effective().membership(),
            "membership "
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
