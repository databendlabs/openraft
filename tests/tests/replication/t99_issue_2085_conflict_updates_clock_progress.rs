use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_response::RpcResponse;
use crate::fixtures::ut_harness;

/// A Conflict response to regular replication proves that the follower accepted the Leader's
/// vote, so it must refresh that follower's clock progress.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn conflict_updates_clock_progress() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 100,
            election_timeout_min: 5_000,
            election_timeout_max: 10_000,
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            allow_log_reversion: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config);

    tracing::info!("--- initialize a 3-voter cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- restart voter 1 with empty state");
    {
        let (n1, _log, _sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(1).unwrap();
        n1.shutdown().await?;
    }

    let (conflict_tx, conflict_rx) = TypeConfig::oneshot();
    let conflict_tx = Arc::new(Mutex::new(Some(conflict_tx)));

    tracing::info!(
        log_index,
        "--- make voter 1 unreachable immediately after its first Conflict"
    );
    {
        router
            .set_rpc_post_hook(RPCTypes::AppendEntries, move |router, _req, resp, _from, target| {
                if target == 1 && matches!(resp, RpcResponse::AppendEntries(resp) if resp.is_conflict()) {
                    router.set_unreachable(target, true);

                    if let Some(tx) = conflict_tx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                }

                Box::pin(async { Ok(()) })
            })
            .await;
    }

    tracing::info!(log_index, "--- trigger regular replication to the empty voter");
    let refresh_started = TypeConfig::now();
    {
        router.new_raft_node(1).await;
        router.client_request(0, "conflict", 1).await?;
        TypeConfig::timeout(Duration::from_secs(1), conflict_rx).await??;
    }

    tracing::info!(log_index, "--- Conflict refreshes voter 1 clock progress");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.wait(timeout())
            .metrics(
                |metrics| {
                    metrics
                        .heartbeat
                        .as_ref()
                        .and_then(|clock| clock.get(&1))
                        .copied()
                        .flatten()
                        .is_some_and(|acked| acked.into_inner() >= refresh_started)
                },
                "Conflict refreshes clock progress",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(1))
}
