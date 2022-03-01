use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use tracing::Instrument;

use crate::fixtures::RaftRouter;

/// Ensures the stale value of ReplicationCore.last_log_id won't affect replication.
/// If `ReplicationCore.last_log_id` is used, the end position of log for loading may underflow the start.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn stale_last_log_id() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();

    async {
        // Setup test dependencies.
        let config = Arc::new(
            Config {
                heartbeat_interval: 50,
                election_timeout_min: 500,
                election_timeout_max: 1000,
                max_payload_entries: 1,
                max_applied_log_to_keep: 0,
                ..Default::default()
            }
            .validate()?,
        );
        let mut router = RaftRouter::new(config.clone());
        router.network_send_delay(5);

        let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {3,4}).await?;

        let n_threads = 4;
        let n_ops = 500;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        for i in 0..n_threads {
            tokio::spawn({
                let router = router.clone();
                let tx = tx.clone();

                async move {
                    router.client_request_many(0, &format!("{}", i), n_ops).await;
                    let _ = tx.send(());
                }
            });
        }

        for _i in 0..n_threads {
            let _ = rx.recv().await;
            log_index += n_ops as u64;
        }

        router.wait(&1, Some(Duration::from_millis(500))).await?.log(Some(log_index), "").await?;
        router.wait(&2, Some(Duration::from_millis(500))).await?.log(Some(log_index), "").await?;
        router.wait(&3, Some(Duration::from_millis(500))).await?.log(Some(log_index), "").await?;
        router.wait(&4, Some(Duration::from_millis(500))).await?.log(Some(log_index), "").await?;

        Ok(())
    }
    .instrument(ut_span)
    .await
}
