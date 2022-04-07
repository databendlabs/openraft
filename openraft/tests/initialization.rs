use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::RaftStorage;
use openraft::State;
use tokio::sync::oneshot;

use crate::fixtures::init_default_ut_tracing;

#[macro_use]
mod fixtures;

/// Cluster initialization test.
///
/// What does this test do?
///
/// - brings 3 nodes online with only knowledge of themselves.
/// - asserts that they remain in learner state with no activity (they should be completely passive).
/// - initializes the cluster with membership config including all nodes.
/// - asserts that the cluster was able to come online, elect a leader and maintain a stable state.
/// - asserts that the leader was able to successfully commit its initial payload and that all followers have
///   successfully replicated the payload.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn initialization() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, timeout(), "empty").await?;
    router.assert_pristine_cluster().await;

    // Sending an external requests will also find all nodes in Learner state.
    //
    // This demonstrates fire-and-forget external request, which will be serialized
    // with other processing. It is not required for the correctness of the test
    //
    // Since the execution of API messages is serialized, even if the request executes
    // some unknown time in the future (due to fire-and-forget semantics), it will
    // properly receive the state before initialization, as that state will appear
    // later in the sequence.
    //
    // Also, this external request will be definitely executed, since it's ordered
    // before other requests in the Raft core API queue, which definitely are executed
    // (since they are awaited).
    for node in [0, 1, 2] {
        router.external_request(node, |s, _sm, _net| assert_eq!(s, State::Learner));
    }

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), timeout(), "init").await?;
    router.assert_stable_cluster(Some(1), Some(log_index)).await;

    for i in 0..3 {
        let mut sto = router.get_storage_handle(&1)?;
        let first = sto.get_log_entries(0..2).await?.first().cloned();

        tracing::info!("--- check membership is replicated: id: {}, first log: {:?}", i, first);
        let mem = match first.unwrap().payload {
            EntryPayload::Membership(ref x) => x.clone(),
            _ => {
                panic!("expect Membership payload")
            }
        };
        assert_eq!(btreeset![0, 1, 2], mem.get_configs()[0].clone());

        let sm_mem = sto.last_applied_state().await?.1;
        assert_eq!(
            EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(0, 0), 0)),
                Membership::new(vec![btreeset! {0,1,2}], None)
            ),
            sm_mem
        );
    }

    // At this time, one of the nodes is the leader, all the others are followers.
    // Check via an external request as well. Again, this is not required for the
    // correctness of the test.
    //
    // This demonstrates how to synchronize on the execution of the external
    // request by using a oneshot channel.
    let mut found_leader = false;
    let mut follower_count = 0;
    for node in [0, 1, 2] {
        let (tx, rx) = oneshot::channel();
        router.external_request(node, |s, _sm, _net| tx.send(s).unwrap());
        match rx.await.unwrap() {
            State::Leader => {
                assert!(!found_leader);
                found_leader = true;
            }
            State::Follower => {
                follower_count += 1;
            }
            s => panic!("Unexpected node {} state: {:?}", node, s),
        }
    }
    assert!(found_leader);
    assert_eq!(2, follower_count);

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
