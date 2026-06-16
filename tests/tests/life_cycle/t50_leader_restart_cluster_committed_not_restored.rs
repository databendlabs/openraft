use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// A leader that restarts and restores leadership without a re-election must not surface a
/// cluster-committed log id that was read back from storage.
///
/// `cluster_committed` is established only by replicating to a quorum; it is never restored from
/// `RaftStorage`. So a restored leader that has lost its quorum reports `local_committed` (the
/// apply ceiling, restored from storage) while `cluster_committed` stays `None`. This guarantees a
/// non-null `cluster_committed` in the metrics is always a genuine, freshly quorum-granted commit
/// — never a stale value resurrected from storage and broadcast to followers.
///
/// Beyond the metric, the test triggers a heartbeat and asserts the broadcast
/// `AppendEntriesRequest.leader_commit` is null, proving the restored commit never reaches the
/// wire.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_restart_cluster_committed_not_restored() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up a 3-node cluster; node-0 is the leader");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- the live leader has an established cluster commit");
    {
        let m = router.get_metrics(&0)?;
        assert_eq!(
            Some(log_index),
            m.cluster_committed.as_ref().map(|x| x.index()),
            "cluster_committed established by quorum replication"
        );
        assert_eq!(Some(log_index), m.local_committed.as_ref().map(|x| x.index()));
    }

    tracing::info!(
        log_index,
        "--- take both followers offline so the leader loses its quorum"
    );
    for id in [1, 2] {
        let (node, _ls, _sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(id).unwrap();
        node.shutdown().await?;
    }

    tracing::info!(
        log_index,
        "--- restart node-0 with its persisted state; it restores leadership"
    );
    {
        let (node, ls, sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(0).unwrap();
        node.shutdown().await?;

        router.new_raft_node_with_sto(0, ls, sm).await;
        router.wait(&0, timeout()).state(ServerState::Leader, "restore leadership without election").await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "applied restored from storage").await?;
    }

    tracing::info!(
        log_index,
        "--- local_committed is restored from storage, cluster_committed is not"
    );
    {
        let m = router.get_metrics(&0)?;

        // The committed value persisted in `RaftStorage` is restored into the local apply ceiling.
        assert_eq!(
            Some(log_index),
            m.local_committed.as_ref().map(|x| x.index()),
            "local_committed restored from storage"
        );

        // Without a quorum the restored leader can never re-establish the cluster commit, so the
        // restored value is never promoted to `cluster_committed` nor broadcast to followers.
        assert_eq!(
            None, m.cluster_committed,
            "cluster_committed must not be restored from storage"
        );
    }

    tracing::info!(
        log_index,
        "--- a triggered heartbeat must broadcast a null leader_commit"
    );
    {
        // Intercept node-0's outgoing AppendEntries (heartbeats use the same RPC) and record the
        // `leader_commit` carried on the wire.
        let total = Arc::new(AtomicU64::new(0));
        let non_null = Arc::new(AtomicU64::new(0));
        {
            let total = total.clone();
            let non_null = non_null.clone();
            router
                .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, from_id, _target| {
                    if from_id == 0
                        && let RpcRequest::AppendEntries(ae) = &req
                    {
                        total.fetch_add(1, Ordering::Relaxed);
                        if ae.leader_commit.is_some() {
                            non_null.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Box::pin(futures::future::ready(Ok::<(), RPCError<TypeConfig, Infallible>>(())))
                })
                .await;
        }

        router.get_raft_handle(&0)?.trigger().heartbeat().await?;

        // Wait until the heartbeat workers have actually emitted the broadcast.
        let mut sent = 0;
        for _ in 0..100 {
            sent = total.load(Ordering::Relaxed);
            if sent > 0 {
                break;
            }
            TypeConfig::sleep(Duration::from_millis(10)).await;
        }
        assert!(sent > 0, "the restored leader must broadcast at least one heartbeat");

        // The commit restored from storage is never put on the wire.
        assert_eq!(
            0,
            non_null.load(Ordering::Relaxed),
            "a restored leader's heartbeat must broadcast a null leader_commit"
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
