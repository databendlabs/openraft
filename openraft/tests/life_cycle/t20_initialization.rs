use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::error::InitializeError;
use openraft::error::NotAllowed;
use openraft::error::NotInMembers;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::RaftStorage;
use openraft::ServerState;
use openraft::Vote;
use tokio::sync::oneshot;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

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
async fn initialization() -> anyhow::Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0);
    router.new_raft_node(1);
    router.new_raft_node(2);

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], ServerState::Learner, timeout(), "empty").await?;
    router.assert_pristine_cluster();

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
        router.external_request(node, |s, _sto, _net| {
            assert_eq!(s.server_state, ServerState::Learner);
        });
    }

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreeset! {0,1,2}).await?;
        log_index += 1;

        for node_id in [0, 1, 2] {
            router.wait(&node_id, timeout()).log(Some(log_index), "init").await?;
        }
    }

    router.assert_stable_cluster(Some(1), Some(log_index));

    tracing::info!("--- check membership state");
    for node_id in [0, 1, 2] {
        router.external_request(node_id, move |s, _sto, _net| {
            let want = EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(0, 0), 0)),
                Membership::new(vec![btreeset! {0,1,2}], None),
            );
            let want = Arc::new(want);
            assert_eq!(
                s.membership_state.effective, want,
                "node-{}: effective membership",
                node_id
            );
            assert_eq!(
                s.membership_state.committed, want,
                "node-{}: committed membership",
                node_id
            );
        });
    }

    for i in [0, 1, 2] {
        let mut sto = router.get_storage_handle(&1)?;
        let first = sto.get_log_entries(0..2).await?.first().cloned();

        tracing::info!("--- check membership is replicated: id: {}, first log: {:?}", i, first);
        let mem = match first.unwrap().payload {
            EntryPayload::Membership(ref x) => x.clone(),
            _ => {
                panic!("expect Membership payload")
            }
        };
        assert_eq!(btreeset![0, 1, 2], mem.get_joint_config()[0].clone());

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
        router.external_request(node, |s, _sm, _net| tx.send(s.server_state).unwrap());
        match rx.await.unwrap() {
            ServerState::Leader => {
                assert!(!found_leader);
                found_leader = true;
            }
            ServerState::Follower => {
                follower_count += 1;
            }
            s => panic!("Unexpected node {} state: {:?}", node, s),
        }
    }
    assert!(found_leader);
    assert_eq!(2, follower_count);

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn initialize_err_target_not_include_target() -> anyhow::Result<()> {
    // Initialize a node with membership config that does not include the target node that accepts the `initialize`
    // request.

    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0);
    router.new_raft_node(1);

    for node in [0, 1] {
        router.external_request(node, |s, _sto, _net| {
            assert_eq!(s.server_state, ServerState::Learner);
        });
    }

    for node_id in 0..2 {
        let n = router.get_raft_handle(&node_id)?;
        let res = n.initialize(btreeset! {9}).await;

        assert!(res.is_err(), "expect error but: {:?}", res);
        let err = res.unwrap_err();

        assert_eq!(
            InitializeError::NotInMembers(NotInMembers {
                node_id,
                membership: Membership::new(vec![btreeset! {9}], None)
            }),
            err
        );
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn initialize_err_not_allowed() -> anyhow::Result<()> {
    // Initialize a node with membership config that does not include the target node that accepts the `initialize`
    // request.

    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0);

    for node in [0] {
        router.external_request(node, |s, _sto, _net| {
            assert_eq!(s.server_state, ServerState::Learner);
        });
    }

    tracing::info!("--- Initialize node 0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreeset! {0}).await?;
    }

    tracing::info!("--- Initialize node 0 again, not allowed");
    {
        let n0 = router.get_raft_handle(&0)?;
        let res = n0.initialize(btreeset! {0}).await;
        assert!(res.is_err(), "expect error but: {:?}", res);
        let err = res.unwrap_err();

        assert_eq!(
            InitializeError::NotAllowed(NotAllowed {
                last_log_id: Some(LogId {
                    leader_id: LeaderId::new(1, 0),
                    index: 1
                }),
                vote: Vote::new_committed(1, 0)
            }),
            err
        );
    }

    Ok(())
}

/// What does this test do?
///   this test is ought to test if the router is aware of network failures
///
/// - initialize 1 node as leader and 2 nodes as follower
/// - set one follower's network to be unreachable
/// - check if router's connecting to unreachable follower returning an `Err`
/// - assert whether the cluster still works properly
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn router_network_failure_aware() -> anyhow::Result<()> {
    let config = Arc::new(Config::default().validate()?);
    let nodes = btreeset! {0, 1, 2};
    let mut router = RaftRouter::new(config.clone());
    let mut log_index = 0;

    // making cluster by hand
    tracing::info!("--- init single node");
    {
        router.new_raft_node(0);
        router.initialize_from_single_node(0).await?;
        log_index += 1;
        router.wait_for_log(&btreeset! {0}, Some(log_index), timeout(), "init single node cluster").await?;
    }

    tracing::info!("--- add reachable learners to cluster");
    {
        router.new_raft_node(1);
        router.add_learner(0, 1).await?;
        log_index += 1;

        router.new_raft_node(2);
        router.add_learner(0, 2).await?;
        log_index += 1;

        router
            .wait_for_log(
                &btreeset! {0},
                Some(log_index),
                timeout(),
                "should add 2 learner the cluster",
            )
            .await?;
    }

    tracing::info!("--- add unreachable learner to cluster");
    {
        router.new_raft_node(3);
        router.block_node(3);
        assert!(router.add_learner(0, 3).await.is_err());

        router
            .wait_for_log(
                &btreeset! {0, 1, 2},
                Some(log_index),
                timeout(),
                "add learner fail, no logs",
            )
            .await?;
    }

    tracing::info!("change membership");
    {
        assert!(router.leader().is_some());
        let leader = router.leader().unwrap();

        let h = router.get_raft_handle(&leader)?;

        h.change_membership(btreeset! {0, 1, 2}, true, false).await?;
        log_index += 2;

        router
            .wait(&0, Some(Duration::from_secs(5)))
            .metrics(|x| x.current_leader == Some(0), "wait for cluster to have a leader")
            .await?;
        router
            .wait_for_log(
                &btreeset! {0},
                Some(log_index),
                timeout(),
                "change membership, commit one log",
            )
            .await?;
    }

    tracing::info!("--- finish making cluster");
    {
        router
            .wait_for_members(
                &btreeset! {0},
                nodes.clone(),
                timeout(),
                format!("cluster should be: {:?}", nodes.clone()).as_str(),
            )
            .await?;
    }

    tracing::info!("--- wait until the cluster stable");
    {
        router
            .wait_for_metrics(
                &1,
                |x| x.current_leader == Some(0),
                timeout(),
                "wait for election complete",
            )
            .await?;
    }

    tracing::info!("--- write 100 logs");
    {
        router.client_request_many(0, "client", 100).await?;
        log_index += 100;
        router.wait_for_log(&btreeset! {0}, Some(log_index), timeout(), "write should complete").await?;
    }

    tracing::info!("--- unplug n2, make it unreachable for router");
    {
        router.block_node(2);
    }

    tracing::info!("--- isolate leader, force an election");
    {
        router.isolate_node(0);
    }

    tracing::info!("--- cluster should unable to commit logs now");
    {
        let write_req = router.client_request(1, "client", 0);
        let timeout = tokio::time::timeout(Duration::from_millis(1000), write_req).await;
        assert!(timeout.is_err(), "now cluster should fail");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
