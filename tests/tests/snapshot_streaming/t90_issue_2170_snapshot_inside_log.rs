use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::storage::RaftLogStorage;
use openraft::testing::blank_ent;
use openraft::testing::membership_ent;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// The leader the test plays: it is never started.
const LEADER: MemNodeId = 0;
const FOLLOWER: MemNodeId = 1;
/// Builds the leader's snapshot, outside the cluster.
const HELPER: MemNodeId = 2;

/// A snapshot that ends inside the follower's log, from the leader that wrote that log, keeps the
/// entries after it and lets them be applied once committed.
///
/// The follower holds `0..=40`, committed through 10, and installs a snapshot at 30. Installing it
/// used to submit log I/O `(leader, 30)` after `(leader, 40)`: a debug build panicked, and a
/// release build applied nothing past 30 until the leader appended a new entry.
///
/// See: <https://github.com/databendlabs/openraft/issues/2170>
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_inside_follower_log() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config);

    let leader_vote = Vote::new_committed(1, LEADER);
    let voters = btreeset! {LEADER, FOLLOWER};
    let mut entries = vec![membership_ent::<TypeConfig>(1, LEADER, 0, vec![voters])];
    entries.extend((1..=40).map(|i| blank_ent::<TypeConfig>(1, LEADER, i)));

    tracing::info!("--- build the leader's snapshot at index 30 on node-{}", HELPER);
    let snapshot = {
        router.new_raft_node(HELPER).await;
        let helper = router.get_raft_handle(&HELPER)?;

        helper
            .append_entries(AppendEntriesRequest {
                vote: leader_vote,
                prev_log_id: None,
                entries: entries[..=30].to_vec(),
                leader_commit: Some(log_id(1, LEADER, 30)),
            })
            .await?;
        helper.wait(timeout()).applied_index(Some(30), "helper applied").await?;

        helper.trigger().snapshot().await?;
        helper.wait(timeout()).snapshot(log_id(1, LEADER, 30), "helper built snapshot").await?;

        helper.get_snapshot().await?.unwrap()
    };

    router.new_raft_node(FOLLOWER).await;
    let follower = router.get_raft_handle(&FOLLOWER)?;

    tracing::info!("--- append 0..=40 to node-{}, committed through 10", FOLLOWER);
    {
        follower
            .append_entries(AppendEntriesRequest {
                vote: leader_vote,
                prev_log_id: None,
                entries,
                leader_commit: Some(log_id(1, LEADER, 10)),
            })
            .await?;
        follower.wait(timeout()).applied_index(Some(10), "applied up to 10").await?;
    }

    tracing::info!("--- install the snapshot at 30 on node-{}, inside its log", FOLLOWER);
    {
        follower.install_full_snapshot(leader_vote, snapshot).await?;

        follower.wait(timeout()).applied_index(Some(30), "snapshot installed").await?;
        follower.wait(timeout()).purged(Some(log_id(1, LEADER, 30)), "purged up to the snapshot").await?;

        let (mut sto, _sm) = router.get_storage_handle(&FOLLOWER)?;
        let log_state = sto.get_log_state().await?;
        assert_eq!(Some(log_id(1, LEADER, 40)), log_state.last_log_id, "31..=40 are kept");
    }

    tracing::info!("--- commit 40 with no new entry: 31..=40 are applied");
    {
        follower
            .append_entries(AppendEntriesRequest {
                vote: leader_vote,
                prev_log_id: Some(log_id(1, LEADER, 40)),
                entries: vec![],
                leader_commit: Some(log_id(1, LEADER, 40)),
            })
            .await?;
        follower.wait(timeout()).applied_index(Some(40), "applied up to 40").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
