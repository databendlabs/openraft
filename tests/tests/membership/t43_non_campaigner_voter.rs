use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::Vote;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::async_runtime::Select2;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::membership::NodeRole;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use openraft_membership::ExplicitChange;
use openraft_membership::ExplicitMembership;

use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = (),
        Membership = ExplicitMembership<u64, ()>,
        MembershipEdit = openraft_membership::ExplicitChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// A voter with only `ReceiveLogs` stores logs and votes, but leads only when the leader transfers
/// leadership to it.
///
/// Any two of voters 1, 2 and 3 form a read and a write quorum. Once node 1 is cut off, node 2
/// needs the vote of node 3 to win, and node 3 as leader commits through node 2.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn non_campaigner_voter_leads_only_through_transfer() -> Result<()> {
    let majorities = [btreeset! {1, 2}, btreeset! {1, 3}, btreeset! {2, 3}];
    let membership = ExplicitMembership::new(majorities.clone(), majorities, btreeset! {1, 2, 3})?;
    let membership = membership.with_roles(btreemap! {3 => NodeRole::ReceiveLogs})?;
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2, 3},
        membership.clone(),
    )
    .await?;

    tracing::info!("--- node 1 wins term 1 and commits x; node 3 stores x");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;

        let response = cluster.raft(1).client_write("x".to_string()).await?;
        assert_eq!(log_id::<TypeConfig>(1, 1, 2), response.log_id);
        cluster.raft(3).wait(timeout()).log_index(Some(2), "node 3 stores x").await?;
    }

    tracing::info!("--- trigger an election on node 3; it starts none, so its vote stays and node 1 still leads");
    {
        cluster.raft(3).trigger().elect(false).await?;
        TypeConfig::sleep(Duration::from_millis(500)).await;

        let n3 = cluster.raft(3);
        let metrics = n3.metrics().borrow_watched().clone();
        assert_eq!(Vote::new_committed(1, 1), metrics.vote);
        assert_eq!(ServerState::Follower, metrics.state);

        let n1 = cluster.raft(1);
        let metrics = n1.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, metrics.state);
    }

    tracing::info!("--- cut node 1 off, and wait until the lease of node 1 runs out");
    {
        cluster.cut_links(btreeset! {(1, 2), (1, 3)});

        // `leader_lease` equals `election_timeout_max`, see `EngineConfig::new`.
        let leader_lease = Duration::from_millis(cluster.raft(1).config().election_timeout_max);
        TypeConfig::sleep(leader_lease + Duration::from_millis(200)).await;
    }

    tracing::info!("--- node 2 wins term 2 with the vote of node 3, and commits its blank entry through node 3");
    {
        cluster.raft(2).trigger().elect(false).await?;
        cluster.raft(2).wait(timeout()).state(ServerState::Leader, "node 2 becomes leader").await?;
        cluster.raft(2).wait(timeout()).applied_index(Some(3), "node 2 commits its blank entry").await?;
    }

    tracing::info!("--- node 2 transfers leadership to node 3, which wins term 3 and commits its blank entry");
    {
        cluster.raft(2).trigger().transfer_leader(3).await?;
        cluster.raft(3).wait(timeout()).state(ServerState::Leader, "node 3 becomes leader").await?;
        cluster.raft(3).wait(timeout()).applied_index(Some(4), "node 3 commits its blank entry").await?;
    }

    tracing::info!("--- node 3 commits y through node 2, and both nodes apply the same entries");
    {
        let response = cluster.raft(3).client_write("y".to_string()).await?;
        assert_eq!(log_id::<TypeConfig>(3, 3, 5), response.log_id);

        let want = vec![
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(0, 0, 0), membership),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(1, 1, 1)),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 2), "x".to_string()),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(2, 2, 3)),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(3, 3, 4)),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(3, 3, 5), "y".to_string()),
        ];
        for node_id in [2, 3] {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(5), "apply y").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

/// A blocking re-add of a voter without `ReceiveLogs` returns once its entry commits, and keeps
/// the voter's role.
///
/// Node 1 needs the vote of node 2 to win, but sends node 2 no log, so replication to node 2 never
/// reaches the re-add entry.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn blocking_re_add_of_voter_without_receive_logs_returns() -> Result<()> {
    let membership = ExplicitMembership::new([btreeset! {1, 2}], [btreeset! {1}], btreeset! {1, 2})?;
    let membership = membership.with_roles(btreemap! {2 => NodeRole::Empty})?;
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2},
        membership.clone(),
    )
    .await?;
    let leader = cluster.raft(1);

    tracing::info!("--- node 1 wins term 1 with the vote of node 2, and commits its blank entry alone");
    {
        leader.trigger().elect(false).await?;
        leader.wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
    }

    tracing::info!(
        log_index = 2,
        "--- a blocking re-add of node 2 returns the committed entry, which keeps every role"
    );
    {
        let adding = leader.add_learner(2, (), true);
        let response = TypeConfig::timeout(WAIT_TIMEOUT, adding).await??;
        assert_eq!(log_id::<TypeConfig>(1, 1, 2), response.log_id);
        assert_eq!(Some(membership), response.membership);
    }

    Ok(())
}

/// A blocking re-add that waits for a learner returns once a membership change takes `ReceiveLogs`
/// from the learner and keeps it in the membership.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn blocking_re_add_returns_once_target_stops_receiving_logs() -> Result<()> {
    let membership = ExplicitMembership::new([btreeset! {1}], [btreeset! {1}], btreeset! {1, 2})?;
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2},
        membership.clone(),
    )
    .await?;
    let leader = cluster.raft(1);

    tracing::info!("--- node 1 wins term 1, and learner 2 stores its blank entry");
    {
        leader.trigger().elect(false).await?;
        leader.wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
        cluster.raft(2).wait(timeout()).log_index(Some(1), "learner 2 stores the blank entry").await?;
    }

    tracing::info!(
        log_index = 2,
        "--- cut learner 2 off; a blocking re-add of it commits, but cannot catch up"
    );
    let mut adding = {
        cluster.cut_links(btreeset! {(1, 2)});

        let mut adding = Box::pin(leader.add_learner(2, (), true));
        let wait = leader.wait(timeout());
        let applied = wait.applied_index(Some(2), "node 1 applies the re-add");
        match TypeConfig::select2(adding.as_mut(), applied).await {
            Select2::First(res) => panic!("the re-add returned while learner 2 is cut off: {res:?}"),
            Select2::Second(res) => res?,
        };
        adding
    };

    tracing::info!(
        log_index = 2,
        "--- the state machine answered the re-add before node 1 reported it applied, \
         so one poll receives the answer and starts the wait for learner 2"
    );
    {
        let polled = futures::poll!(adding.as_mut());
        assert!(polled.is_pending(), "the re-add waits for learner 2 to catch up");
    }

    tracing::info!(
        log_index = 3,
        "--- take ReceiveLogs from learner 2; the pending re-add returns its committed entry"
    );
    {
        let change = ExplicitChange::default().with_roles(btreemap! {2 => NodeRole::Empty});
        let response = leader.change_membership(change, true).await?;
        assert_eq!(log_id::<TypeConfig>(1, 1, 3), response.log_id);
        let want = membership.clone().with_roles(btreemap! {2 => NodeRole::Empty})?;
        assert_eq!(Some(want), response.membership);

        let response = TypeConfig::timeout(WAIT_TIMEOUT, adding).await??;
        assert_eq!(log_id::<TypeConfig>(1, 1, 2), response.log_id);
        assert_eq!(Some(membership), response.membership);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
