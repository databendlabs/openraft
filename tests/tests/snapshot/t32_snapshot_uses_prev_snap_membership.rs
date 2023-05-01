use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::storage::RaftLogReaderExt;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::Membership;
use openraft::SnapshotPolicy;
use openraft::StorageHelper;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

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

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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

        router
            .wait_for_log(
                &btreeset![0, 1],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                timeout(),
                "1st snapshot",
            )
            .await?;

        {
            let logs = sto0.get_log_entries(..).await?;
            assert_eq!(3, logs.len(), "only one applied log is kept");
        }
        let m = StorageHelper::new(&mut sto0, &mut sm0).get_membership().await?;

        assert_eq!(
            &Membership::new(vec![btreeset! {0,1}], None),
            m.committed().membership(),
            "membership "
        );
        assert_eq!(
            &Membership::new(vec![btreeset! {0,1}], None),
            m.effective().membership(),
            "membership "
        );
    }

    tracing::info!(log_index, "--- send just enough logs to trigger the 2nd snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold * 2 - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold * 2 - 1;

        router.wait_for_log(&btreeset![0, 1], Some(log_index), None, "send log to trigger snapshot").await?;
        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                None,
                "2nd snapshot",
            )
            .await?;
    }

    tracing::info!(log_index, "--- check membership");
    {
        {
            let logs = sto0.get_log_entries(..).await?;
            assert_eq!(3, logs.len(), "only one applied log");
        }
        let m = StorageHelper::new(&mut sto0, &mut sm0).get_membership().await?;

        assert_eq!(
            &Membership::new(vec![btreeset! {0,1}], None),
            m.committed().membership(),
            "membership "
        );
        assert_eq!(
            &Membership::new(vec![btreeset! {0,1}], None),
            m.effective().membership(),
            "membership "
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
