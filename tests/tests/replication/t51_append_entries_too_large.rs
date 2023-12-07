use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::PayloadTooLarge;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::RPCTypes;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// If append-entries returns PayloadTooLarge, Openraft should split the request into smaller
/// chunks.
/// In this test, RaftNetwork::append_entries() returns PayloadTooLarge if the number of entries is
/// greater than 1.
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_entries_too_large() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster of 1 node");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let n = 10u64;

    tracing::info!(log_index, "--- write {} entries to leader", n);
    {
        log_index += router.client_request_many(0, "0", n as usize).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), format!("{} writes", n)).await?;
    }

    let count = Arc::new(AtomicU64::new(0));
    let count_small = Arc::new(AtomicU64::new(0));

    let ac = count.clone();
    let sac = count_small.clone();

    tracing::info!(log_index, "--- node-1 accepts only 1 entry rpc");
    {
        router.set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, _id, target| {
            let r: AppendEntriesRequest<_> = req.try_into().unwrap();
            if target == 1 {
                ac.fetch_add(1, Ordering::Relaxed);

                if r.entries.len() > 1 {
                    return Err(PayloadTooLarge::new_entries_hint(1).into());
                }

                sac.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        });
    }

    tracing::info!(log_index, "--- add node-1 as learner");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await?;
        log_index += 1;

        router.wait(&1, timeout()).applied_index(Some(log_index), "1 node added").await?;
    }

    assert_eq!(
        15,
        count.load(Ordering::Relaxed),
        "13 logs: M,B,normal*10,M; 2 failed RPC due to too-large, because each hint is used 10 times"
    );
    assert_eq!(13, count_small.load(Ordering::Relaxed), "13 logs: M,B,normal*10,M");

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
