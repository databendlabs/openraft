use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::impls::ChangeMembers;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// How long a case watches to see that no purge runs.
const NO_PURGE: Duration = Duration::from_millis(300);

/// A leader that removes a learner closes the learner's replication stream, while a log read of
/// the stream still runs on the transport's task. The leader purges no log until the read has
/// returned: the purge waits for the read, then runs, and the read succeeds.
///
/// The log store fails a held read whose logs were purged while it waited, as a store fails whose
/// purge removes what a running read uses, and RaftCore would stop on that storage error.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn purge_waits_for_the_read_of_a_removed_learners_stream() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        // Purge only what this case asks for, and all of it.
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };
    let mut router = RaftRouter::builder(config)?.pulled_append().build();

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (log0, _sm0) = router.get_storage_handle(&0)?;

    tracing::info!(
        log_index,
        "--- add learner 3: node 0's stream to it reads node 0's log from the start, and the read is held"
    );
    let read = {
        // The voters have every log through `log_index`, so only the stream to node 3 reads one.
        let read = log0.pause_reads_up_to(log_index);
        router.new_raft_node(3).await?;
        n0.add_learner(3, (), false).await?;
        log_index += 1;
        read.entered().await;
        read
    };

    tracing::info!(
        log_index,
        "--- remove learner 3: node 0 closes the stream while its read waits"
    );
    {
        n0.change_membership(ChangeMembers::RemoveNodes(btreeset! {3}), false).await?;
        log_index += 1;
    }

    let snapshot = log_id(1, 0, log_index);

    tracing::info!(
        log_index,
        "--- node 0 snapshots its log and asks to purge it: the purge waits for the read"
    );
    {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(snapshot, "node 0 built the snapshot").await?;
        n0.trigger().purge_log(log_index).await?;

        let purged = router.wait_with_timeout(&0, NO_PURGE).purged(Some(snapshot), "no purge yet").await;
        expect_timeout(purged)?;
    }

    tracing::info!(log_index, "--- release the read: node 0 purges, and still leads");
    {
        read.release();
        router.wait(&0).purged(Some(snapshot), "node 0 purges once the read returned").await?;
        router.wait(&0).state(ServerState::Leader, "node 0 still leads").await?;
    }

    Ok(())
}

/// A leader that steps down closes its replication streams, while a log read of one still runs on
/// the transport's task. As a follower, the node purges no log until the read has returned: the
/// purge waits for the read, then runs, and the read succeeds.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn purge_waits_for_the_read_of_a_stream_closed_by_stepping_down() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        // Purge only what this case asks for, and all of it.
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };
    let mut router = RaftRouter::builder(config)?.pulled_append().build();

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (log0, _sm0) = router.get_storage_handle(&0)?;

    tracing::info!(
        log_index,
        "--- add learner 3: node 0's stream to it reads node 0's log from the start, and the read is held"
    );
    let read = {
        // The voters have every log through `log_index`, so only the stream to node 3 reads one.
        let read = log0.pause_reads_up_to(log_index);
        router.new_raft_node(3).await?;
        n0.add_learner(3, (), false).await?;
        log_index += 1;
        read.entered().await;
        read
    };

    tracing::info!(
        log_index,
        "--- node 0 hands leadership to node 1 and closes its streams while the read waits"
    );
    {
        n0.trigger().transfer_leader(1).await?;
        router.wait(&1).state(ServerState::Leader, "node 1 leads").await?;
        log_index += 1;
        router.wait(&0).applied_index(Some(log_index), "node 0 applies node 1's first log").await?;
        router.wait(&0).current_leader(1, "node 0 follows node 1").await?;
    }

    let snapshot = log_id(2, 1, log_index);

    tracing::info!(
        log_index,
        "--- node 0 snapshots its log and asks to purge it: the purge waits for the read"
    );
    {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(snapshot, "node 0 built the snapshot").await?;
        n0.trigger().purge_log(log_index).await?;

        let purged = router.wait_with_timeout(&0, NO_PURGE).purged(Some(snapshot), "no purge yet").await;
        expect_timeout(purged)?;
    }

    tracing::info!(
        log_index,
        "--- release the read: node 0 purges, and still follows node 1"
    );
    {
        read.release();
        router.wait(&0).purged(Some(snapshot), "node 0 purges once the read returned").await?;
        router.wait(&0).current_leader(1, "node 0 still follows node 1").await?;
    }

    Ok(())
}
