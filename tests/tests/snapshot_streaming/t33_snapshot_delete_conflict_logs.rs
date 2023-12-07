use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use openraft::testing;
use openraft::testing::blank_ent;
use openraft::testing::log_id;
use openraft::testing::membership_ent;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftSnapshotBuilder;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::StorageHelper;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Installing snapshot on a node that has logs conflict with snapshot.meta.last_log_id will delete
/// all conflict logs.
///
/// - Feed logs to node-0 to build a snapshot.
/// - Init node-1 with conflicting log.
/// - Send snapshot to node-1 to override its conflicting logs.
/// - ensure that snapshot overrides the existent membership and conflicting logs are deleted.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_delete_conflicting_logs() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index;

    tracing::info!("--- manually init node-0 with a higher vote, in order to override conflict log on learner later");
    {
        let (mut sto0, sm0) = router.new_store();

        // When the node starts, it will become candidate and increment its vote to (5,0)
        sto0.save_vote(&Vote::new(4, 0)).await?;
        testing::blocking_append(&mut sto0, [
            // manually insert the initializing log
            membership_ent(0, 0, 0, vec![btreeset! {0}]),
        ])
        .await?;
        log_index = 1;

        router.new_raft_node_with_sto(0, sto0, sm0).await;

        router.wait(&0, timeout()).state(ServerState::Leader, "init node-0 server-state").await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "init node-0 log").await?;
    }

    tracing::info!(log_index, "--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

        router.wait(&0, timeout()).applied_index(Some(log_index), "trigger snapshot").await?;
        router
            .wait(&0, timeout())
            .snapshot(LogId::new(CommittedLeaderId::new(5, 0), log_index), "build snapshot")
            .await?;
    }

    tracing::info!(log_index, "--- create node-1 and add conflicting logs");
    {
        router.new_raft_node(1).await;

        let req = AppendEntriesRequest {
            vote: Vote::new_committed(1, 0),
            prev_log_id: None,
            entries: vec![
                blank_ent(0, 0, 0),
                blank_ent(1, 0, 1),
                // conflict membership will be replaced with membership in snapshot
                Entry {
                    log_id: LogId::new(CommittedLeaderId::new(1, 0), 2),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {2,3}], None)),
                },
                blank_ent(1, 0, 3),
                blank_ent(1, 0, 4),
                blank_ent(1, 0, 5),
                blank_ent(1, 0, 6),
                blank_ent(1, 0, 7),
                blank_ent(1, 0, 8),
                blank_ent(1, 0, 9),
                blank_ent(1, 0, 10),
                // another conflict membership, will be removed
                Entry {
                    log_id: LogId::new(CommittedLeaderId::new(1, 0), 11),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {4,5}], None)),
                },
            ],
            leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
        };
        let option = RPCOption::new(Duration::from_millis(1_000));

        router.new_client(1, &()).await.append_entries(req, option).await?;

        tracing::info!(log_index, "--- check that learner membership is affected");
        {
            let (mut sto1, mut sm1) = router.get_storage_handle(&1)?;
            let m = StorageHelper::new(&mut sto1, &mut sm1).get_membership().await?;

            tracing::info!("got membership of node-1: {:?}", m);
            assert_eq!(
                &Membership::new(vec![btreeset! {2,3}], None),
                m.committed().membership()
            );
            assert_eq!(
                &Membership::new(vec![btreeset! {4,5}], None),
                m.effective().membership()
            );
        }
    }

    tracing::info!(log_index, "--- manually build and install snapshot to node-1");
    {
        let (mut sto0, mut sm0) = router.get_storage_handle(&0)?;

        let snap = {
            let mut b = sm0.get_snapshot_builder().await;
            b.build_snapshot().await?
        };

        let req = InstallSnapshotRequest {
            vote: sto0.read_vote().await?.unwrap(),
            meta: snap.meta.clone(),
            offset: 0,
            data: snap.snapshot.into_inner(),
            done: true,
        };

        let option = RPCOption::new(Duration::from_millis(1_000));

        router.new_client(1, &()).await.install_snapshot(req, option).await?;

        tracing::info!(log_index, "--- DONE installing snapshot");

        router.wait(&1, timeout()).snapshot(log_id(5, 0, log_index), "node-1 snapshot").await?;
    }

    tracing::info!(
        log_index,
        "--- check that learner membership is affected, conflict log are deleted"
    );
    {
        let (mut sto1, mut sm1) = router.get_storage_handle(&1)?;

        let m = StorageHelper::new(&mut sto1, &mut sm1).get_membership().await?;

        tracing::info!("got membership of node-1: {:?}", m);
        assert_eq!(
            &Membership::new(vec![btreeset! {0}], None),
            m.committed().membership(),
            "membership should be overridden by the snapshot"
        );
        assert_eq!(
            &Membership::new(vec![btreeset! {0}], None),
            m.effective().membership(),
            "conflicting effective membership does not have to be clear"
        );

        let log_st = sto1.get_log_state().await?;
        assert_eq!(
            Some(LogId::new(CommittedLeaderId::new(5, 0), snapshot_threshold - 1)),
            log_st.last_purged_log_id,
            "purge up to last log id in snapshot"
        );
        assert_eq!(
            Some(LogId::new(CommittedLeaderId::new(5, 0), snapshot_threshold - 1)),
            log_st.last_log_id,
            "reverted to last log id in snapshot"
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
