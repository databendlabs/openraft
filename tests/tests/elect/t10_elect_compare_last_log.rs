use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::Vote;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::blank_ent;
use openraft::testing::membership_ent;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// The last_log in a vote request must be greater or equal than the local one.
///
/// - Fake a cluster with two node: with last log {2,1} and {1,1}.
/// - Bring up the cluster and only node 0 can become leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn elect_compare_last_log() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let (mut sto0, sm0) = router.new_store();
    let (mut sto1, sm1) = router.new_store();

    tracing::info!("--- fake store: sto0: last log: 2,1");
    {
        sto0.save_vote(&Vote::new(10, 0)).await?;

        sto0.blocking_append([
            //
            blank_ent(0, 0, 0),
            membership_ent(2, 0, 1, vec![btreeset! {0,1}]),
        ])
        .await?;
    }

    tracing::info!("--- fake store: sto1: last log: 1,2");
    {
        sto1.save_vote(&Vote::new(10, 0)).await?;

        sto1.blocking_append([
            blank_ent(0, 0, 0),
            membership_ent(1, 0, 1, vec![btreeset! {0,1}]),
            blank_ent(1, 0, 2),
        ])
        .await?;
    }

    tracing::info!("--- bring up cluster and elect");

    router.new_raft_node_with_sto(0, sto0.clone(), sm0.clone()).await;
    router.new_raft_node_with_sto(1, sto1.clone(), sm1.clone()).await;

    router.wait(&0, timeout()).state(ServerState::Leader, "only node 0 becomes leader").await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
