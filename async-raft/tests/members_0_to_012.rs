use std::sync::Arc;

use anyhow::Result;
use async_raft::Config;
use fixtures::RaftRouter;
use futures::stream::StreamExt;
use maplit::btreeset;

mod fixtures;

/// A leader must wait for non-voter to commit member-change from [0] to [0,1,2].
/// There is a bug that leader commit a member change log directly because it only checks
/// the replication of voters and believes itself is the only voter.
///
/// - Init 1 leader and 2 non-voter.
/// - Isolate non-voter.
/// - Asserts that membership change won't success.
///
/// RUST_LOG=async_raft,memstore,members_0_to_012=trace cargo test -p async-raft --test members_0_to_012
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn members_0_to_012() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    // Assert all nodes are in non-voter state & have no entries.
    let want;

    // router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want = 1;

    router.wait_for_log(&btreeset![0], want, None, "init node 0").await?;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_non_voter(0, 1));
    new_nodes.push(router.add_non_voter(0, 2));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }

    router.wait_for_log(&btreeset![0], want, None, "init node 0").await?;

    tracing::info!("--- isolate node 1,2, so that membership [0,1,2] wont commit");

    router.isolate_node(1).await;
    router.isolate_node(2).await;

    tracing::info!("--- changing cluster config, should timeout");

    tokio::spawn({
        let router = router.clone();
        async move {
            let _x = router.change_membership(0, btreeset! {0,1,2}).await;
        }
    });

    let res = router
        .wait_for_metrics(
            &0,
            |x| x.last_applied > want,
            None,
            "the next joint log should not commit",
        )
        .await;
    assert!(res.is_err(), "joint log should not commit");

    Ok(())
}
