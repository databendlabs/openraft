#![allow(clippy::result_large_err)]

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyerror::AnyError;
use anyhow::bail;
use anyhow::Result;
use maplit::btreeset;
use openraft::error::NetworkError;
use openraft::raft::InstallSnapshotRequest;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::RPCTypes;
use tokio::time::sleep;
use tokio::time::Instant;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// When sending a snapshot fails transiently, retry should fetch the latest snapshot instead of
/// keeping the old one forever.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_retry_uses_latest_snapshot() -> Result<()> {
    let config = Arc::new(
        Config {
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            snapshot_max_chunk_size: 1,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    let leader = router.get_raft_handle(&0)?;
    let learner = router.get_raft_handle(&1)?;

    tracing::info!(
        log_index,
        "--- isolate learner, build and purge the 1st snapshot on leader"
    );
    {
        router.set_network_error(1, true);

        log_index += router.client_request_many(0, "0", 10).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "leader applies 10 logs").await?;

        leader.trigger().snapshot().await?;
        leader
            .wait(timeout())
            .snapshot(
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                "leader builds 1st snapshot",
            )
            .await?;
        leader
            .wait(timeout())
            .purged(
                Some(LogId::new(CommittedLeaderId::new(1, 0), log_index)),
                "leader purges logs in 1st snapshot",
            )
            .await?;
    }

    let seen_snapshot_ids = Arc::new(Mutex::new(BTreeSet::new()));
    let allow_complete = Arc::new(AtomicBool::new(false));

    tracing::info!(
        log_index,
        "--- restore learner and keep failing snapshot chunks after offset 0"
    );
    {
        let seen_snapshot_ids = seen_snapshot_ids.clone();
        let allow_complete = allow_complete.clone();

        router.set_rpc_pre_hook(RPCTypes::InstallSnapshot, move |_router, req, _id, target| {
            if target != 1 {
                return Ok(());
            }

            let req: InstallSnapshotRequest<_> = req.try_into().unwrap();
            seen_snapshot_ids.lock().unwrap().insert(req.meta.snapshot_id.clone());

            if !allow_complete.load(Ordering::Relaxed) && req.offset > 0 {
                let any_err = AnyError::error("inject snapshot chunk network error");
                return Err(NetworkError::new(&any_err).into());
            }

            Ok(())
        });

        router.set_network_error(1, false);
    }

    wait_for_snapshot_ids(&seen_snapshot_ids, 1).await?;

    let rpc_count_before = install_snapshot_rpc_count(&router);
    sleep(Duration::from_millis(250)).await;
    let rpc_count_after = install_snapshot_rpc_count(&router);
    assert!(
        rpc_count_after - rpc_count_before <= 12,
        "install_snapshot retries should back off instead of tight-looping: before={}, after={}",
        rpc_count_before,
        rpc_count_after
    );

    tracing::info!(
        log_index,
        "--- build the 2nd snapshot while the 1st snapshot keeps retrying"
    );
    {
        log_index += router.client_request_many(0, "0", 5).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "leader applies another 5 logs").await?;

        leader.trigger().snapshot().await?;
        leader
            .wait(timeout())
            .snapshot(
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                "leader builds 2nd snapshot",
            )
            .await?;
    }

    wait_for_snapshot_ids(&seen_snapshot_ids, 2).await?;

    tracing::info!(
        log_index,
        "--- allow snapshot transfer to finish and expect learner to install the 2nd snapshot"
    );
    allow_complete.store(true, Ordering::Relaxed);

    learner
        .wait(timeout())
        .snapshot(
            LogId::new(CommittedLeaderId::new(1, 0), log_index),
            "learner installs the latest snapshot",
        )
        .await?;

    router.rpc_pre_hook(RPCTypes::InstallSnapshot, None);

    Ok(())
}

async fn wait_for_snapshot_ids(snapshot_ids: &Arc<Mutex<BTreeSet<String>>>, want: usize) -> Result<()> {
    let deadline = Instant::now() + timeout().unwrap();

    loop {
        let ids = snapshot_ids.lock().unwrap().clone();
        if ids.len() >= want {
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!("timed out waiting for {want} distinct snapshot ids, seen {ids:?}");
        }

        sleep(Duration::from_millis(50)).await;
    }
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}

fn install_snapshot_rpc_count(router: &RaftRouter) -> u64 {
    let counts = router.get_rpc_count();
    *counts.get(&RPCTypes::InstallSnapshot).unwrap_or(&0)
}
