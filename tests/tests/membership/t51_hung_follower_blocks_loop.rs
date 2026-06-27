use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::TypedRaftRouter;
use crate::fixtures::ut_harness;

/// How long an AppendEntries RPC to the removed follower stays in flight.
const HANG: Duration = Duration::from_secs(30);

async fn setup() -> Result<TypedRaftRouter> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    // After the cluster is formed, make every AppendEntries to node 2 hang.
    router
        .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, _req, _from, target| {
            // let hang = target == 2;
            Box::pin(async move {
                if target == 2 {
                    TypeConfig::sleep(HANG).await;
                }
                Ok::<(), RPCError<TypeConfig, Infallible>>(())
            })
        })
        .await;

    // A write commits via the {0,1} quorum and leaves node 2's replication task
    // parked inside the hung AppendEntries RPC.
    router.client_request(0, "before-remove", 1).await?;
    log_index += 1;
    router
        .wait(&0, timeout())
        .applied_index(Some(log_index), "write commits via the {0,1} quorum")
        .await?;

    Ok(router)
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_hung_follower_must_not_block_raft_core_loop_1() -> Result<()> {
    let router = setup().await?;

    // Remove the hung follower.
    let leader = router.get_raft_handle(&0)?;
    let membership_change = leader.change_membership([0, 1], false);
    TypeConfig::timeout(Duration::from_secs(10), membership_change)
        .await
        .expect("membership change completed within the window")?;

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_hung_follower_must_not_block_raft_core_loop_2() -> Result<()> {
    let router = setup().await?;

    // Remove the hung follower in separate task.
    let leader = router.get_raft_handle(&0)?;
    let _membership = TypeConfig::spawn(async move {
        let _ = leader.change_membership([0, 1], false).await;
    });

    // HACK: wait for the leader to reach `close_membership()` to await the replication task.
    TypeConfig::sleep(Duration::from_millis(500)).await;

    // The leader should still serve the surviving quorum.
    let write = router.client_request(0, "after-remove", 2);
    TypeConfig::timeout(Duration::from_secs(10), write)
        .await
        .expect("write completed within the window")?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
