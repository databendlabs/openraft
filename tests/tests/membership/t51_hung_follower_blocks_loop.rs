use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::OperationGate;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

async fn setup() -> Result<(RaftRouter, OperationGate)> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    let gate = OperationGate::default();
    let rpc_gate = gate.clone();

    // Hold AppendEntries to node 2 until each scenario explicitly releases it.
    router
        .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
            let gate = rpc_gate.clone();
            async move {
                if call.to == 2 {
                    gate.block().await;
                }
                Ok::<(), RPCError<TypeConfig, Infallible>>(())
            }
        })
        .await;

    // A write commits via the {0,1} quorum and leaves node 2's replication task
    // parked inside the hung AppendEntries RPC.
    router.client_request(0, "before-remove", 1).await?;
    log_index += 1;
    router.wait(&0).applied_index(Some(log_index), "write commits via the {0,1} quorum").await?;

    TypeConfig::timeout(timeout().unwrap(), gate.entered()).await?;
    Ok((router, gate))
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_hung_follower_must_not_block_raft_core_loop_1() -> Result<()> {
    let (router, gate) = setup().await?;

    // Remove the hung follower.
    let leader = router.get_raft_handle(&0)?;
    let membership_change = leader.change_membership([0, 1], false);
    TypeConfig::timeout(Duration::from_secs(10), membership_change)
        .await
        .expect("membership change completed within the window")?;

    gate.release();
    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_hung_follower_must_not_block_raft_core_loop_2() -> Result<()> {
    let (router, gate) = setup().await?;

    // Remove the hung follower in separate task.
    let leader = router.get_raft_handle(&0)?;
    let membership = TypeConfig::spawn(async move { leader.change_membership([0, 1], false).await });

    tracing::info!("--- wait until the removal changes the leader's membership");
    router.wait(&0).voter_ids([0, 1], "membership removal processed").await?;

    // Refresh the quorum lease after waiting for the core loop to reach the suspected blocking
    // point. Heartbeats are disabled in this test, so the previous lease may have expired.
    let refresh_started = TypeConfig::now();
    router.get_raft_handle(&0)?.trigger().heartbeat().await?;
    router
        .wait(&0)
        .leader_with_quorum_acked(
            Some(refresh_started),
            "leader lease recovered through the surviving quorum",
        )
        .await?;

    // The leader should still serve the surviving quorum.
    let write = router.client_request(0, "after-remove", 2);
    TypeConfig::timeout(Duration::from_secs(10), write)
        .await
        .expect("write completed within the window")?;

    gate.release();
    membership.await??;
    Ok(())
}
