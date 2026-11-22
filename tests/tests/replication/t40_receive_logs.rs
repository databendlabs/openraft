use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::EntryPayload;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::RaftError;
use openraft::errors::WriteVoterWithoutReceiveLogs;
use openraft::membership::NodeRole;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;

use crate::fixtures::custom_membership::MinimalQuorums;
use crate::fixtures::custom_membership::MinimalQuorumsMembership;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = (),
        Membership = MinimalQuorumsMembership,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// Only log receivers get entries, including after roles change; a read voter still gets
/// heartbeats.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn replication_respects_receive_logs() -> Result<()> {
    let campaigner = NodeRole::Campaigner | NodeRole::ReceiveLogs;
    let initial = MinimalQuorumsMembership {
        read: MinimalQuorums(vec![btreeset! {1, 2}]),
        write: MinimalQuorums(vec![btreeset! {1}]),
        roles: btreemap! {
            2 => NodeRole::Empty,
            3 => NodeRole::ReceiveLogs,
            4 => NodeRole::Empty,
            5 => NodeRole::Campaigner,
            6 => campaigner,
        },
    };
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2, 3, 4, 5, 6},
        initial.clone(),
    )
    .await?;
    let leader = cluster.raft(1);

    tracing::info!("--- node 1 wins the read quorum with node 2 and replicates only to log receivers");
    let first_entries = {
        leader.trigger().elect(false).await?;
        leader.wait(timeout()).applied_index(Some(1), "commit the blank entry").await?;
        leader.client_write("before-change".to_string()).await?;
        let want = vec![
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(0, 0, 0), initial.clone()),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(1, 1, 1)),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 2), "before-change".to_string()),
        ];
        for node_id in [1, 3, 6] {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(2), "apply the initial write").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
        want
    };

    tracing::info!("--- acknowledge a heartbeat from read voter 2 while excluded nodes store no replicated entries");
    {
        leader.trigger().heartbeat().await?;
        leader
            .wait(timeout())
            .metrics(
                |metrics| {
                    let Some(heartbeat) = &metrics.heartbeat else {
                        return false;
                    };
                    let Some(acked) = heartbeat.get(&2) else {
                        return false;
                    };
                    acked.is_some()
                },
                "read voter 2 acknowledges a heartbeat",
            )
            .await?;

        let heartbeat_interval = Duration::from_millis(leader.config().heartbeat_interval);
        TypeConfig::sleep(heartbeat_interval * 2).await;
        for node_id in [2, 4, 5] {
            let metrics = cluster.raft(node_id).metrics().borrow_watched().clone();
            assert_eq!(Some(0), metrics.last_log_index, "node {node_id}");
        }
    }

    tracing::info!("--- remove ReceiveLogs from nodes 3 and 6 and grant it to nodes 4 and 5");
    let changed = {
        let mut changed = initial.clone();
        changed.roles.insert(3, NodeRole::Empty);
        changed.roles.insert(4, NodeRole::ReceiveLogs);
        changed.roles.insert(5, campaigner);
        changed.roles.insert(6, NodeRole::Campaigner);
        leader.append_membership(changed.clone(), EntryPayload::Blank, []).await?;
        changed
    };

    tracing::info!("--- new receivers catch up and apply the next write; former receivers keep their old log");
    {
        let response = leader.client_write("after-change".to_string()).await?;
        let expected_log_id = log_id::<TypeConfig>(1, 1, 4);
        assert_eq!(expected_log_id, response.log_id);

        let mut want = first_entries.clone();
        want.push(EntryOf::<TypeConfig>::new_membership(
            log_id::<TypeConfig>(1, 1, 3),
            changed,
        ));
        want.push(EntryOf::<TypeConfig>::new_normal(
            expected_log_id,
            "after-change".to_string(),
        ));
        for node_id in [1, 4, 5] {
            cluster
                .raft(node_id)
                .wait(timeout())
                .applied_index(Some(4), "apply the write after changing roles")
                .await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }

        let heartbeat_interval = Duration::from_millis(leader.config().heartbeat_interval);
        TypeConfig::sleep(heartbeat_interval * 2).await;
        for node_id in [3, 6] {
            let applied = cluster.applied(node_id);
            assert_eq!(first_entries, applied, "node {node_id}");
        }
        let read_voter = cluster.raft(2).metrics().borrow_watched().clone();
        assert_eq!(Some(0), read_voter.last_log_index);
    }

    Ok(())
}

/// A node that loses `ReceiveLogs` and gets it back catches up: the leader drops the work in
/// flight of the stream it closed, and replicates through the new stream.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn regained_receive_logs_resumes_replication() -> Result<()> {
    let with_logs = MinimalQuorumsMembership {
        read: MinimalQuorums(vec![btreeset! {1}]),
        write: MinimalQuorums(vec![btreeset! {1}]),
        roles: btreemap! { 2 => NodeRole::ReceiveLogs },
    };
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2},
        with_logs.clone(),
    )
    .await?;
    let leader = cluster.raft(1);

    tracing::info!("--- node 1 leads, and node 2 applies its first write through a stream in flight");
    {
        leader.trigger().elect(false).await?;
        leader.wait(timeout()).applied_index(Some(1), "commit the blank entry").await?;
        leader.client_write("first".to_string()).await?;
        cluster.raft(2).wait(timeout()).applied_index(Some(2), "node 2 applies the first write").await?;
    }

    tracing::info!("--- take ReceiveLogs from node 2, then write while node 2 gets no logs");
    let without_logs = {
        let mut without_logs = with_logs.clone();
        without_logs.roles.insert(2, NodeRole::Empty);
        leader.append_membership(without_logs.clone(), EntryPayload::Blank, []).await?;
        leader.client_write("excluded".to_string()).await?;
        without_logs
    };

    tracing::info!("--- give ReceiveLogs back to node 2: it catches up through the next write");
    {
        leader.append_membership(with_logs.clone(), EntryPayload::Blank, []).await?;
        let response = leader.client_write("restored".to_string()).await?;
        let expected_log_id = log_id::<TypeConfig>(1, 1, 6);
        assert_eq!(expected_log_id, response.log_id);

        cluster
            .raft(2)
            .wait(timeout())
            .applied_index(Some(6), "node 2 applies the write after the restore")
            .await?;
        let want = vec![
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(0, 0, 0), with_logs.clone()),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(1, 1, 1)),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 2), "first".to_string()),
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(1, 1, 3), without_logs),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 4), "excluded".to_string()),
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(1, 1, 5), with_logs),
            EntryOf::<TypeConfig>::new_normal(expected_log_id, "restored".to_string()),
        ];
        assert_eq!(want, cluster.applied(2));
    }

    Ok(())
}

/// A write voter without `ReceiveLogs` never acknowledges an entry, so `append_membership()`
/// rejects a membership that has one.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn append_membership_rejects_write_voter_without_receive_logs() -> Result<()> {
    let initial = MinimalQuorumsMembership {
        read: MinimalQuorums(vec![btreeset! {1, 2}]),
        write: MinimalQuorums(vec![btreeset! {1, 2}]),
        roles: btreemap! {},
    };
    let cluster =
        TestCluster::<TypeConfig>::start(cluster_config(SnapshotPolicy::Never), btreeset! {1, 2}, initial.clone())
            .await?;
    let leader = cluster.raft(1);

    tracing::info!("--- elect node 1 and commit its blank entry, so that it may append a membership");
    {
        leader.trigger().elect(false).await?;
        leader.wait(timeout()).applied_index(Some(1), "commit the blank entry").await?;
    }

    tracing::info!("--- take ReceiveLogs from write voter 2: append_membership() rejects the membership");
    {
        let mut proposed = initial;
        proposed.roles.insert(2, NodeRole::Campaigner);
        let err = leader.append_membership(proposed, EntryPayload::Blank, []).await.unwrap_err();

        let no_logs = WriteVoterWithoutReceiveLogs { node_id: 2 };
        let want =
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::WriteVoterWithoutReceiveLogs(no_logs));
        assert_eq!(RaftError::APIError(want), err);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
