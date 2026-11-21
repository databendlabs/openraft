use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::OperationGate;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// A log stream whose plan the leader drops must not outlive the purge of its entries. The leader
/// purges the entries the plan held back only once the replication task confirms that it no longer
/// reads them, and the stream then sends nothing more, although the leader has no other plan for
/// the follower yet.
///
/// The events, with the indexes of this test:
///
/// 1. Node 1 has matched through index 5, and its stream generates an append with entry 6 that the
///    transport holds. Nodes 0 and 2 commit through index 16 without it. Node 0 takes a snapshot at
///    index 16 and asks to purge through it, which node 1's plan, `LogsSince { prev: 5 }`, holds
///    back.
/// 2. Node 1 restarts with an empty log. A heartbeat at index 5 gets a conflict, and with
///    `allow_next_revert` node 0 drops node 1's plan and asks its replication task to clear it.
///    Node 0 does not purge yet: the task confirms only once the stream session that holds the
///    append has ended.
/// 3. The transport's hold is released. Node 1 rejects entry 6, which ends the session, and the
///    task confirms the clear. Only then does node 0 purge through index 16. Node 1 installs the
///    snapshot at index 16, but its answer is held, so node 0 has no new plan for node 1.
/// 4. Entries 7 to 9 are purged, so a stream that went on would send an append with no entries
///    after index 6, which node 1 accepts because index 6 lies below the commit point its snapshot
///    set, and repeat it forever. The stream sends nothing more instead, because its plan was
///    cleared.
/// 5. Once node 0 learns that node 1 installed the snapshot, it plans the entries after index 16,
///    and node 1 receives entries 17 to 19 without a leader change.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stale_stream_after_follower_revert_gives_way() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        // The stale stream's next read stays inside the purged range.
        max_payload_entries: 3,
        // Purge only what this test asks for.
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    let stream_gate = OperationGate::default();
    let appends_to_1 = Arc::new(AtomicU64::new(0));

    tracing::info!(
        log_index,
        "--- count the appends from node 0 to node 1, and hold those with entries"
    );
    {
        let gate = stream_gate.clone();
        let appends = appends_to_1.clone();
        router
            .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
                let to_node_1 = call.from == 0 && call.to == 1;
                if to_node_1 {
                    appends.fetch_add(1, Ordering::Relaxed);
                }
                let carries_entries = match &call.request {
                    RpcRequest::AppendEntries(append) => !append.entries.is_empty(),
                    _ => false,
                };
                let hold = to_node_1 && carries_entries;
                let gate = gate.clone();
                async move {
                    if hold {
                        gate.block().await;
                    }
                    Ok::<(), RPCError<TypeConfig, Infallible>>(())
                }
            })
            .await;
    }

    tracing::info!(log_index, "--- node 1's stream takes the next write and is held");
    {
        router.client_request(0, "foo", 1).await?;
        log_index += 1;
        stream_gate.entered().await;
    }

    tracing::info!(
        log_index,
        "--- write more entries, which only node 2 receives, and snapshot them on node 0"
    );
    {
        let written = router.client_request_many(0, "foo", 10).await?;
        log_index += written.count;
        router.wait(&2).applied_index(Some(log_index), "node 2 has the entries").await?;

        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "node 0 built the snapshot").await?;

        // Postponed while node 1's stream still has the entries in flight.
        n0.trigger().purge_log(log_index).await?;
    }

    tracing::info!(log_index, "--- wipe node 1 and let node 0 accept its log reversion");
    {
        router.stop_node(1).await?;
        router.new_raft_node(1).await?;
        n0.trigger().allow_next_revert(&1, true).await??;
    }

    let snapshot_gate = OperationGate::default();

    tracing::info!(
        log_index,
        "--- hold node 1's answer to the snapshot, so node 0 makes no new plan for node 1"
    );
    {
        let gate = snapshot_gate.clone();
        router
            .set_rpc_post_hook(RpcKind::InstallFullSnapshot, move |call| {
                let hold = call.from == 0 && call.to == 1;
                let gate = gate.clone();
                async move {
                    if hold {
                        gate.block().await;
                    }
                    Ok(())
                }
            })
            .await;
    }

    tracing::info!(
        log_index,
        "--- a heartbeat finds node 1 empty: node 0 clears node 1's plan, but must not purge while the stream holds its append"
    );
    {
        n0.trigger().heartbeat().await?;
        router
            .wait(&0)
            .metrics(
                |m| m.replication.as_ref().and_then(|r| r.get(&1)).is_some_and(|matched| matched.is_none()),
                "node 0 reset the progress of node 1",
            )
            .await?;

        let purged = router
            .wait_with_timeout(&0, Duration::from_millis(200))
            .purged(
                Some(log_id(1, 0, log_index)),
                "node 0 waits for node 1's stream to confirm the clear",
            )
            .await;
        expect_timeout(purged)?;
    }

    tracing::info!(
        log_index,
        "--- release the transport's hold: the stream ends and confirms the clear, node 0 purges, and node 1 installs the snapshot"
    );
    {
        stream_gate.release();

        router.wait(&0).purged(Some(log_id(1, 0, log_index)), "node 0 purged its log").await?;
        router.wait(&1).snapshot(log_id(1, 0, log_index), "node 1 installed the snapshot").await?;
        snapshot_gate.entered().await;
    }

    tracing::info!(
        log_index,
        "--- with its plan cleared, the stream must send nothing more"
    );
    {
        let sent = appends_to_1.load(Ordering::Relaxed);

        // The replication task delivers commit 16, which no request has carried yet, with one
        // entry-less request. A stream that went on would send one every few milliseconds.
        TypeConfig::sleep(Duration::from_millis(200)).await;
        let sent_later = appends_to_1.load(Ordering::Relaxed) - sent;
        assert!(
            sent_later <= 1,
            "expect at most the commit request after the stream ends, got {} appends",
            sent_later
        );
    }

    tracing::info!(
        log_index,
        "--- release the snapshot answer; node 1 must receive the entries written after it"
    );
    {
        snapshot_gate.release();

        let written = router.client_request_many(0, "foo", 3).await?;
        log_index += written.count;
        router.wait(&1).applied_index(Some(log_index), "node 1 has the fresh entries").await?;

        let metrics = n0.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, metrics.state);
        assert_eq!(1, metrics.current_term);
    }

    Ok(())
}
