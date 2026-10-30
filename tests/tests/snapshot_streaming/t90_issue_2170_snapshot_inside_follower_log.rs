use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::blank_ent;
use openraft::testing::membership_ent;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Regression test for <https://github.com/databendlabs/openraft/issues/2170>.
///
/// A full snapshot whose last log id lies past the follower's committed log id but inside its log
/// must not move the follower's log I/O progress back.
///
/// The follower holds entries `0..=40` from the leader `(1, 0)`, committed through 10. It then
/// installs a snapshot at 30 from the same leader. `install_full_snapshot` accepts it because 30 is
/// above the committed log id, and keeps `accepted` at 40. Before the fix, `RaftCore` then
/// submitted log I/O `(leader, 30)` while `submitted` was `(leader, 40)`: debug builds panicked in
/// `IOProgress::advance`, and release builds moved `submitted` back below `flushed` and stopped
/// applying at 30.
///
/// The test acts as the leader, node 0, and builds the snapshot on a second follower that holds
/// the same entries `0..=30`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_inside_follower_log_does_not_move_log_progress_back() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    let leader_vote = Vote::new_committed(1, 0);
    let entries = {
        let mut entries = vec![membership_ent::<TypeConfig>(1, 0, 0, vec![btreeset! {0, 1, 2}])];
        entries.extend((1..=40).map(|index| blank_ent::<TypeConfig>(1, 0, index)));
        entries
    };

    tracing::info!("--- build a snapshot at log index 30 on node 2");
    let snapshot = {
        router.new_raft_node(2).await?;
        let n2 = router.get_raft_handle(&2)?;

        n2.append_entries(AppendEntriesRequest {
            vote: leader_vote,
            prev_log_id: None,
            entries: entries[..=30].to_vec(),
            leader_commit: Some(log_id(1, 0, 30)),
        })
        .await?;
        router.wait(&2).applied_index(Some(30), "node 2 applied 30").await?;

        n2.trigger().snapshot().await?;
        router.wait(&2).snapshot(log_id(1, 0, 30), "node 2 snapshot at 30").await?;
        n2.get_snapshot().await?.expect("node 2 has a snapshot")
    };

    tracing::info!("--- node 1 holds log 0..=40, committed through 10");
    let n1 = {
        router.new_raft_node(1).await?;
        let n1 = router.get_raft_handle(&1)?;

        n1.append_entries(AppendEntriesRequest {
            vote: leader_vote,
            prev_log_id: None,
            entries: entries.clone(),
            leader_commit: Some(log_id(1, 0, 10)),
        })
        .await?;
        router.wait(&1).applied_index(Some(10), "node 1 applied 10").await?;
        n1
    };

    tracing::info!("--- install the snapshot at 30 on node 1, inside its log ending at 40");
    {
        n1.install_full_snapshot(leader_vote, snapshot).await?;
        router.wait(&1).snapshot(log_id(1, 0, 30), "node 1 installed snapshot at 30").await?;
    }

    tracing::info!("--- commit through 40 on node 1; log 31..=40 must still be applied");
    {
        n1.append_entries(AppendEntriesRequest {
            vote: leader_vote,
            prev_log_id: Some(log_id(1, 0, 40)),
            entries: vec![],
            leader_commit: Some(log_id(1, 0, 40)),
        })
        .await?;
        router.wait(&1).applied_index(Some(40), "node 1 applied 40").await?;
    }

    Ok(())
}
