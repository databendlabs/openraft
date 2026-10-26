use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Ensures the stale value of ReplicationCore.last_log_id won't affect replication.
/// If `ReplicationCore.last_log_id` is used, the end position of log for loading may underflow the
/// start.
///
/// TODO(xp): `max_applied_log_to_keep` to be 0 makes it very easy to enter snapshot replication and
/// it will keeps           replicating every log by snapshot and get timeout.
///           Thus it is disabled until we find another way to test it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
#[ignore]
async fn stale_last_log_id() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            election_timeout_min: 500,
            election_timeout_max: 1000,
            max_payload_entries: 1,
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(5);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3,4}).await?;

    let n_threads: u64 = 4;
    let n_ops = 500;
    let (tx, mut rx) = TypeConfig::mpsc(n_threads as usize);

    for i in 0..n_threads {
        TypeConfig::spawn({
            let router = router.clone();
            let tx = tx.clone();

            async move {
                let client_id = format!("{}", i);
                let result = router.client_request_many(0, &client_id, n_ops).await;
                tx.send(result).await.expect("test receives every workload result");
            }
        });
    }

    for _i in 0..n_threads {
        let result = TypeConfig::timeout(crate::fixtures::WAIT_TIMEOUT, rx.recv()).await?;
        let written = result.expect("client task dropped its sender")?;
        assert_eq!(n_ops as u64, written.count);
        log_index += n_ops as u64;
    }

    router.wait_with_timeout(&1, Duration::from_millis(1000)).applied_index(Some(log_index), "").await?;
    router.wait_with_timeout(&2, Duration::from_millis(1000)).applied_index(Some(log_index), "").await?;
    router.wait_with_timeout(&3, Duration::from_millis(1000)).applied_index(Some(log_index), "").await?;
    router.wait_with_timeout(&4, Duration::from_millis(1000)).applied_index(Some(log_index), "").await?;

    Ok(())
}
