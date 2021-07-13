mod fixtures;

use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::AppendEntriesRequest;
use async_raft::raft::Entry;
use async_raft::raft::EntryConfigChange;
use async_raft::raft::EntryPayload;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::RaftNetwork;
use async_raft::RaftStorage;
use async_raft::SnapshotPolicy;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::hashset;

/// Test membership info is sync correctly along with snapshot.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - ensure that snapshot overrides the existent membership on the non-voter.
///
/// export RUST_LOG=async_raft,memstore,snapshot_overrides_membership=trace
/// cargo test -p async-raft --test snapshot_overrides_membership
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_overrides_membership() -> Result<()> {
    fixtures::init_tracing();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config::build("test".into())
            .snapshot_policy(SnapshotPolicy::LogsSinceLast(snapshot_threshold))
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&hashset![0], want, None, "empty").await?;
        router.wait_for_state(&hashset![0], State::NonVoter, None, "empty").await?;
        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&hashset![0], want, None, "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(want)).await;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
        want = snapshot_threshold;

        router.wait_for_log(&hashset![0], want, None, "send log to trigger snapshot").await?;
        router.assert_stable_cluster(Some(1), Some(want)).await;

        router.wait_for_snapshot(&hashset![0], LogId { term: 1, index: want }, None, "snapshot").await?;
        router
            .assert_storage_state(
                1,
                want,
                Some(0),
                want,
                Some((want.into(), 1, MembershipConfig {
                    members: hashset![0],
                    members_after_consensus: None,
                })),
            )
            .await;
    }

    tracing::info!("--- create non-voter");
    {
        tracing::info!("--- create non-voter");
        router.new_raft_node(1).await;
        let sto = router.get_storage_handle(&1).await?;

        tracing::info!("--- add a membership config log to the non-voter");
        {
            let req = AppendEntriesRequest {
                term: 1,
                leader_id: 0,
                prev_log_id: Default::default(),
                entries: vec![Entry {
                    log_id: LogId { term: 1, index: 1 },
                    payload: EntryPayload::ConfigChange(EntryConfigChange {
                        membership: MembershipConfig {
                            members: hashset![2, 3],
                            members_after_consensus: None,
                        },
                    }),
                }],
                leader_commit: 0,
            };
            router.append_entries(1, req).await?;

            tracing::info!("--- check that non-voter membership is affected");
            {
                let m = sto.get_membership_config().await?;
                assert_eq!(
                    MembershipConfig {
                        members: hashset![2, 3],
                        members_after_consensus: None
                    },
                    m
                );
            }
        }

        tracing::info!("--- add non-voter to the cluster to receive snapshot, which overrides the non-voter storage");
        {
            router.add_non_voter(0, 1).await.expect("failed to add new node as non-voter");

            router.wait_for_log(&hashset![0, 1], want, None, "add non-voter").await?;
            let expected_snap = Some((want.into(), 1, MembershipConfig {
                members: hashset![0u64],
                members_after_consensus: None,
            }));
            router.wait_for_snapshot(&hashset![1], LogId { term: 1, index: want }, None, "").await?;
            router.assert_storage_state(1, want, None /* non-voter does not vote */, want, expected_snap).await;

            let m = sto.get_membership_config().await?;
            assert_eq!(
                MembershipConfig {
                    members: hashset![0],
                    members_after_consensus: None
                },
                m,
                "membership should be overridden by the snapshot"
            );
        }
    }

    Ok(())
}
