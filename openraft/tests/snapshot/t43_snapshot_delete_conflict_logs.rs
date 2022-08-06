use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::RaftSnapshotBuilder;
use openraft::RaftStorage;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::StorageHelper;
use openraft::Vote;

use crate::fixtures::blank;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Installing snapshot on a node that has logs conflict with snapshot.meta.last_log_id will delete all conflict logs.
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
            max_applied_log_to_keep: 0,
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
        let mut sto0 = router.new_store();

        // When the node starts, it will become candidate and increment its vote to (5,0)
        sto0.save_vote(&Vote::new(4, 0)).await?;
        sto0.append_to_log(&[
            // manually insert the initializing log
            &Entry {
                log_id: LogId::new(LeaderId::new(0, 0), 0),
                payload: EntryPayload::Membership(Membership::new(vec![btreeset! {0}], None)),
            },
        ])
        .await?;
        log_index = 1;

        router.new_raft_node_with_sto(0, sto0);

        router.wait(&0, timeout()).state(ServerState::Leader, "init node-0 server-state").await?;
        router.wait(&0, timeout()).log(Some(log_index), "init node-0 log").await?;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

        router.wait(&0, timeout()).log(Some(log_index), "trigger snapshot").await?;
        router
            .wait(&0, timeout())
            .snapshot(LogId::new(LeaderId::new(5, 0), log_index), "build snapshot")
            .await?;
    }

    tracing::info!("--- create node-1 and add conflicting logs");
    {
        router.new_raft_node(1);

        let req = AppendEntriesRequest {
            vote: Vote::new_committed(1, 0),
            prev_log_id: None,
            entries: vec![
                blank(0, 0),
                blank(1, 1),
                // conflict membership will be replaced with membership in snapshot
                Entry {
                    log_id: LogId::new(LeaderId::new(1, 0), 2),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {2,3}], None)),
                },
                blank(1, 3),
                blank(1, 4),
                blank(1, 5),
                blank(1, 6),
                blank(1, 7),
                blank(1, 8),
                blank(1, 9),
                blank(1, 10),
                // another conflict membership, will be removed
                Entry {
                    log_id: LogId::new(LeaderId::new(1, 0), 11),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {4,5}], None)),
                },
            ],
            leader_commit: Some(LogId::new(LeaderId::new(0, 0), 0)),
        };
        router.connect(1, &()).await?.send_append_entries(req).await?;

        tracing::info!("--- check that learner membership is affected");
        {
            let mut sto1 = router.get_storage_handle(&1)?;
            let m = StorageHelper::new(&mut sto1).get_membership().await?;

            tracing::info!("got membership of node-1: {:?}", m);
            assert_eq!(Membership::new(vec![btreeset! {2,3}], None), m.committed.membership);
            assert_eq!(Membership::new(vec![btreeset! {4,5}], None), m.effective.membership);
        }
    }

    tracing::info!("--- manually build and install snapshot to node-1");
    {
        let mut sto0 = router.get_storage_handle(&0)?;

        let snap = {
            let mut b = sto0.get_snapshot_builder().await;
            let snap = b.build_snapshot().await?;
            snap
        };

        let req = InstallSnapshotRequest {
            vote: sto0.read_vote().await?.unwrap(),
            meta: snap.meta.clone(),
            offset: 0,
            data: snap.snapshot.into_inner(),
            done: true,
        };

        router.connect(1, &()).await?.send_install_snapshot(req).await?;

        tracing::info!("--- DONE installing snapshot");

        router
            .wait(&1, timeout())
            .snapshot(LogId::new(LeaderId::new(5, 0), log_index), "node-1 snapshot")
            .await?;
    }

    tracing::info!("--- check that learner membership is affected, conflict log are deleted");
    {
        let mut sto1 = router.get_storage_handle(&1)?;

        let m = StorageHelper::new(&mut sto1).get_membership().await?;

        tracing::info!("got membership of node-1: {:?}", m);
        assert_eq!(
            Membership::new(vec![btreeset! {0}], None),
            m.committed.membership,
            "membership should be overridden by the snapshot"
        );
        assert_eq!(
            Membership::new(vec![btreeset! {0}], None),
            m.effective.membership,
            "membership should be overridden by the snapshot"
        );

        let log_st = sto1.get_log_state().await?;
        assert_eq!(
            Some(LogId::new(LeaderId::new(5, 0), snapshot_threshold - 1)),
            log_st.last_log_id,
            "reverted to last log id in snapshot"
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
