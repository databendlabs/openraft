use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft_memstore::BlockOperation;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A follower acknowledgement that arrives while RaftCore drains client messages is processed
/// before every queued append finishes.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn replication_progress_preempts_the_raft_msg_drain() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            election_timeout_min: 5_000,
            election_timeout_max: 6_000,
            api_batch_capacity: 1,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up a 3-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- slow down appends on the leader only");
    {
        let (_sto, sm) = router.get_storage_handle(&0)?;
        sm.block.set_blocking(BlockOperation::DelayAppend, APPEND_DELAY);
    }

    tracing::info!(log_index, "--- queue writes and observe the first commit");
    {
        for i in 0..WRITES {
            n0.client_write_ff(ClientRequest::make_request("fairness", i), None).await?;
        }

        let first = log_index + 1;
        log_index += WRITES;

        // The first acknowledgement arrives while a later append is delayed. It should be
        // processed well before all five serial appends can finish.
        let early = APPEND_DELAY.mul_f64(4.0);
        router.wait(&0, Some(early)).committed_index(Some(first), "first write committed").await?;

        for id in [0, 1, 2] {
            router.wait(&id, timeout()).applied_index(Some(log_index), "all writes applied").await?;
        }
    }

    Ok(())
}

const APPEND_DELAY: Duration = Duration::from_millis(500);
const WRITES: u64 = 5;

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
