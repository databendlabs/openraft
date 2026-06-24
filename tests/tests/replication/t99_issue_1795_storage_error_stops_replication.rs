use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::SnapshotPolicy;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn storage_error_stops_replication() -> Result<()> {
    let snapshot_threshold = 20;
    let live_log_index = snapshot_threshold + 11;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- build a leader with a purged prefix and a live log suffix");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;
    router.client_request_many(0, "snapshot", (snapshot_threshold - 1 - log_index) as usize).await?;
    log_index = snapshot_threshold - 1;

    router.wait(&0, timeout()).applied_index(Some(log_index), "snapshot trigger entries").await?;
    router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "leader has built snapshot").await?;
    router
        .wait(&0, timeout())
        .purged(Some(log_id(1, 0, log_index)), "leader has purged snapshot logs")
        .await?;

    router.client_request_many(0, "suffix", (live_log_index - log_index) as usize).await?;
    log_index = live_log_index;
    router.wait(&0, timeout()).applied_index(Some(log_index), "leader has live suffix").await?;

    let sent_malformed = Arc::new(AtomicBool::new(false));
    {
        let sent_malformed = sent_malformed.clone();
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, _id, target| {
                let malformed = match req {
                    RpcRequest::AppendEntries(append) if target == 1 => {
                        append.prev_log_id.is_none()
                            && append.entries.first().is_some_and(|entry| entry.log_id.index() > 0)
                    }
                    _ => false,
                };

                let res = if malformed {
                    sent_malformed.store(true, Ordering::SeqCst);
                    Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(
                        "malformed append-entries",
                    )))
                } else {
                    Ok(())
                };

                Box::pin(futures::future::ready(res))
            })
            .await;
    }

    tracing::info!("--- inject one storage read error and add a fresh learner");
    router.new_raft_node(1).await;
    router.set_fail_next_limited_get(&0, true)?;

    let leader = router.get_raft_handle(&0)?;
    leader.add_learner(1, (), false).await?;

    TypeConfig::sleep(Duration::from_millis(800)).await;

    assert!(
        !sent_malformed.load(Ordering::SeqCst),
        "replication retried after a storage error and sent prev_log_id=None with entries after a purged prefix"
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
