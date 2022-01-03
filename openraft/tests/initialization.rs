use std::sync::Arc;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::raft::EntryPayload;
use openraft::raft::Membership;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::LogId;
use openraft::RaftStorage;
use openraft::State;

#[macro_use]
mod fixtures;

/// Cluster initialization test.
///
/// What does this test do?
///
/// - brings 3 nodes online with only knowledge of themselves.
/// - asserts that they remain in non-voter state with no activity (they should be completely passive).
/// - initializes the cluster with membership config including all nodes.
/// - asserts that the cluster was able to come online, elect a leader and maintain a stable state.
/// - asserts that the leader was able to successfully commit its initial payload and that all followers have
///   successfully replicated the payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initialization() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut want = 0;

    // Assert all nodes are in non-voter state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], want, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0, 1, 2], want, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(want)).await;

    for i in 0..3 {
        let sto = router.get_storage_handle(&1).await?;
        let first = sto.get_log_entries(1..2).await?.first().cloned();

        tracing::info!("--- check membership is replicated: id: {}, first log: {:?}", i, first);
        let mem = match first.unwrap().payload {
            EntryPayload::Membership(ref x) => x.clone(),
            _ => {
                panic!("expect Membership payload")
            }
        };
        assert_eq!(btreeset![0, 1, 2], mem.get_ith_config(0).cloned().unwrap());

        let sm_mem = sto.last_applied_state().await?.1;
        assert_eq!(
            Some(EffectiveMembership {
                log_id: LogId { term: 1, index: 1 },
                membership: Membership::new_single(btreeset! {0,1,2})
            }),
            sm_mem
        );
    }

    Ok(())
}
