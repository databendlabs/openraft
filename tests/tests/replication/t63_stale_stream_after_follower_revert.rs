use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft_memstore::OperationGate;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// A log stream that took its plan before a follower lost its log must give way once the leader
/// has purged the entries that plan still had to send.
///
/// The events, with the indexes of this test:
///
/// 1. Node 1 has matched through index 5, and its stream generates an append with entry 6 that the
///    transport holds. Nodes 0 and 2 commit through index 16 without it. Node 0 takes a snapshot at
///    index 16 and asks to purge through it, which node 1's plan, `LogsSince { prev: 5 }`, holds
///    back.
/// 2. Node 1 restarts with an empty log. A heartbeat at index 5 gets a conflict, and with
///    `allow_next_revert` node 0 drops node 1's plan. The purge runs, and node 1 installs the
///    snapshot at index 16 that a separate task sends.
/// 3. The held stream resumes, and node 1 accepts entry 6 as part of its snapshot. The stream's
///    next batch, entries 7 to 9, is purged, so the stream sends an append with no entries after
///    index 6, and node 1 accepts it as well, because index 6 lies below the commit point its
///    snapshot set. The success moves the stream no further than index 6, and node 0 ignores it,
///    because it answers the dropped plan.
/// 4. Node 0's last log id stays above index 6, so the stream never waits, which is where it used
///    to look for a new plan. Repeating step 3, it would never give entry 17 to node 1. It must end
///    at its next request instead, so that the stream planned after the snapshot sends entries 17
///    to 19 without a leader change.
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

    let gate = OperationGate::default();

    tracing::info!(log_index, "--- hold every append with entries from node 0 to node 1");
    {
        let hook_gate = gate.clone();
        router
            .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
                let carries_entries = match &call.request {
                    RpcRequest::AppendEntries(append) => !append.entries.is_empty(),
                    _ => false,
                };
                let hold = call.from == 0 && call.to == 1 && carries_entries;
                let gate = hook_gate.clone();
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
        gate.entered().await;
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

    tracing::info!(
        log_index,
        "--- a heartbeat finds node 1 empty: node 0 purges, and node 1 installs the snapshot"
    );
    {
        n0.trigger().heartbeat().await?;
        router.wait(&0).purged(Some(log_id(1, 0, log_index)), "node 0 purged its log").await?;
        router.wait(&1).snapshot(log_id(1, 0, log_index), "node 1 installed the snapshot").await?;
    }

    tracing::info!(
        log_index,
        "--- release the held stream; node 1 must receive the entries written after it"
    );
    {
        gate.release();

        let written = router.client_request_many(0, "foo", 3).await?;
        log_index += written.count;
        router.wait(&1).applied_index(Some(log_index), "node 1 has the fresh entries").await?;

        let metrics = n0.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, metrics.state);
        assert_eq!(1, metrics.current_term);
    }

    Ok(())
}
