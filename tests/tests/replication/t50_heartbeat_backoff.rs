use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Heartbeats to an unreachable follower back off, and reach it again once it is reachable.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn heartbeat_backoff() -> Result<()> {
    // Node 2 hears no heartbeat while it is unreachable, so with elections it would campaign in
    // a higher term and depose the leader once it is reachable again.
    let config = Config {
        heartbeat_interval: 50,
        election_timeout_min: 150,
        election_timeout_max: 300,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a 3-node cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(
        log_index,
        "--- make node 2 unreachable and count the heartbeats it is sent in 1 s"
    );
    let sent = {
        let sent = Arc::new(AtomicU64::new(0));
        let counter = sent.clone();
        router
            .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
                let res = if call.to == 2 {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(
                        "unreachable",
                    )))
                } else {
                    Ok(())
                };
                futures::future::ready(res)
            })
            .await;

        TypeConfig::sleep(Duration::from_millis(1_000)).await;
        sent.load(Ordering::Relaxed)
    };

    // A heartbeat goes out every 50 ms. The delays of the default `Config::backoff` start at
    // 200 ms, which `election_timeout_min` cuts to 150 ms, so about 7 of the 20 heartbeats of 1 s
    // are sent.
    assert!(
        (2..=10).contains(&sent),
        "heartbeats to an unreachable node must back off, sent: {sent}"
    );

    tracing::info!(log_index, sent, "--- make node 2 reachable; heartbeats reach it again");
    {
        let now = TypeConfig::now();
        router.clear_rpc_pre_hook(RpcKind::AppendEntries).await;

        let leader = router.get_raft_handle(&0)?;
        leader
            .wait(timeout())
            .metrics(
                |metrics| {
                    let heartbeat = metrics.heartbeat.as_ref().expect("metrics of the leader have heartbeats");
                    let Some(Some(acked)) = heartbeat.get(&2) else {
                        return false;
                    };
                    **acked >= now
                },
                "node 2 acknowledges a heartbeat",
            )
            .await?;
    }

    Ok(())
}
