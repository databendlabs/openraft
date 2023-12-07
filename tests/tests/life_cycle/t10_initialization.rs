#![allow(clippy::single_element_loop)]

use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::error::InitializeError;
use openraft::error::NotAllowed;
use openraft::error::NotInMembers;
use openraft::storage::RaftLogReaderExt;
use openraft::storage::RaftStateMachine;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::ServerState;
use openraft::StoredMembership;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Cluster initialization test.
///
/// What does this test do?
///
/// - brings 3 nodes online with only knowledge of themselves.
/// - asserts that they remain in learner state with no activity (they should be completely
///   passive).
/// - initializes the cluster with membership config including all nodes.
/// - asserts that the cluster was able to come online, elect a leader and maintain a stable state.
/// - asserts that the leader was able to successfully commit its initial payload and that all
///   followers have successfully replicated the payload.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn initialization() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], ServerState::Learner, timeout(), "empty").await?;

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
        router.external_request(node, |s| {
            assert_eq!(s.server_state, ServerState::Learner);
        });
    }

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!(log_index, "--- initializing cluster");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreeset! {0,1,2}).await?;
        log_index += 1;

        for node_id in [0, 1, 2] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "init").await?;
        }
    }

    tracing::info!(log_index, "--- check membership state");
    for node_id in [0, 1, 2] {
        router.external_request(node_id, move |s| {
            let want = EffectiveMembership::new(
                Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
                Membership::new(vec![btreeset! {0,1,2}], None),
            );
            let want = Arc::new(want);
            assert_eq!(
                s.membership_state.effective(),
                &want,
                "node-{}: effective membership",
                node_id
            );
            assert_eq!(
                s.membership_state.committed(),
                &want,
                "node-{}: committed membership",
                node_id
            );
        });
    }

    for i in [0, 1, 2] {
        let (mut sto, mut sm) = router.get_storage_handle(&1)?;
        let first = sto.get_log_entries(0..2).await?.into_iter().next();

        tracing::info!(
            log_index,
            "--- check membership is replicated: id: {}, first log: {:?}",
            i,
            first
        );
        let mem = match first.unwrap().payload {
            EntryPayload::Membership(ref x) => x.clone(),
            _ => {
                panic!("expect Membership payload")
            }
        };
        assert_eq!(btreeset![0, 1, 2], mem.get_joint_config()[0].clone());

        let sm_mem = sm.applied_state().await?.1;
        assert_eq!(
            StoredMembership::new(
                Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
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
        let server_state = router.with_raft_state(node, |s| s.server_state).await?;
        match server_state {
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
    // Initialize a node with membership config that does not include the target node that accepts
    // the `initialize` request.

    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    for node in [0, 1] {
        router.external_request(node, |s| {
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
            err.into_api_error().unwrap()
        );
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn initialize_err_not_allowed() -> anyhow::Result<()> {
    // Initialize a node with membership config that does not include the target node that accepts
    // the `initialize` request.

    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    for node in [0] {
        router.external_request(node, |s| {
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
                    leader_id: CommittedLeaderId::new(1, 0),
                    index: 1
                }),
                vote: Vote::new_committed(1, 0)
            }),
            err.into_api_error().unwrap()
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
