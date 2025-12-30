use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::metrics::MetricsRecorder;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A simple MetricsRecorder implementation for testing.
///
/// Captures all metric calls so they can be verified in tests.
#[derive(Debug, Default)]
pub struct TestRecorder {
    // Histograms (sum of all recorded values)
    pub apply_batch_total: AtomicU64,
    pub append_batch_total: AtomicU64,
    pub write_batch_total: AtomicU64,

    // Gauges (last recorded value)
    pub current_term: AtomicU64,
    pub last_log_index: AtomicU64,
    pub applied_index: AtomicU64,
    pub snapshot_index: AtomicU64,
    pub purged_index: AtomicU64,
    pub server_state: AtomicU64,

    // Counters
    pub vote_count: AtomicU64,
    pub heartbeat_count: AtomicU64,
    pub append_count: AtomicU64,
}

impl MetricsRecorder for TestRecorder {
    fn record_apply_batch(&self, n: u64) {
        self.apply_batch_total.fetch_add(n, Ordering::Relaxed);
    }

    fn record_append_batch(&self, n: u64) {
        self.append_batch_total.fetch_add(n, Ordering::Relaxed);
    }

    fn record_write_batch(&self, n: u64) {
        self.write_batch_total.fetch_add(n, Ordering::Relaxed);
    }

    fn set_current_term(&self, v: u64) {
        self.current_term.store(v, Ordering::Relaxed);
    }

    fn set_last_log_index(&self, v: u64) {
        self.last_log_index.store(v, Ordering::Relaxed);
    }

    fn set_applied_index(&self, v: u64) {
        self.applied_index.store(v, Ordering::Relaxed);
    }

    fn set_snapshot_index(&self, v: u64) {
        self.snapshot_index.store(v, Ordering::Relaxed);
    }

    fn set_purged_index(&self, v: u64) {
        self.purged_index.store(v, Ordering::Relaxed);
    }

    fn set_server_state(&self, v: u8) {
        self.server_state.store(v as u64, Ordering::Relaxed);
    }

    fn increment_vote(&self) {
        self.vote_count.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_heartbeat(&self) {
        self.heartbeat_count.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_append(&self) {
        self.append_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Test that all MetricsRecorder methods are called during normal Raft operations.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn test_metrics_recorder_all_fields() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(5),
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    // Create and install the recorder on the leader
    let recorder = Arc::new(TestRecorder::default());
    let leader_id = router.leader().expect("cluster should have a leader");
    let leader = router.get_raft_handle(&leader_id)?;
    leader.set_metrics_recorder(Some(recorder.clone())).await?;

    // --- Trigger client writes ---
    // This should trigger:
    // - record_write_batch (leader batching client writes)
    // - record_append_batch (leader appending to log)
    // - record_apply_batch (applying to state machine)
    // - record_replicate_batch (leader replicating to followers)
    tracing::info!(log_index, "--- write 10 client requests");
    log_index += router.client_request_many(leader_id, "test", 10).await?;

    // Wait for all nodes to apply - this ensures replication completed
    for node_id in [0, 1, 2] {
        router.wait(&node_id, timeout()).applied_index(Some(log_index), "applied").await?;
    }

    // --- Trigger heartbeat ---
    // This should trigger increment_heartbeat
    tracing::info!(log_index, "--- trigger heartbeat");
    leader.trigger().heartbeat().await?;

    // --- Trigger snapshot ---
    // This should trigger set_snapshot_index
    tracing::info!(log_index, "--- trigger snapshot");
    leader.trigger().snapshot().await?;
    router.wait(&leader_id, timeout()).snapshot(log_id(1, leader_id, log_index), "snapshot").await?;

    // --- Trigger purge ---
    // This should trigger set_purged_index
    tracing::info!(log_index, "--- trigger purge");
    leader.trigger().purge_log(log_index).await?;
    router.wait(&leader_id, timeout()).purged(Some(log_id(1, leader_id, log_index)), "purged").await?;

    // Wait a bit for metrics to be recorded
    TypeConfig::sleep(Duration::from_millis(100)).await;

    // --- Verify metrics ---
    tracing::info!("--- verifying metrics");

    // Histograms: should have recorded some batches
    let write_batch = recorder.write_batch_total.load(Ordering::Relaxed);
    let append_batch = recorder.append_batch_total.load(Ordering::Relaxed);
    let apply_batch = recorder.apply_batch_total.load(Ordering::Relaxed);

    tracing::info!(write_batch, append_batch, apply_batch, "histogram metrics");

    assert!(write_batch > 0, "write_batch should be recorded, got {}", write_batch);
    assert!(
        append_batch > 0,
        "append_batch should be recorded, got {}",
        append_batch
    );
    assert!(apply_batch > 0, "apply_batch should be recorded, got {}", apply_batch);

    // Gauges: should reflect current state
    let term = recorder.current_term.load(Ordering::Relaxed);
    let last_log = recorder.last_log_index.load(Ordering::Relaxed);
    let applied = recorder.applied_index.load(Ordering::Relaxed);
    let snapshot = recorder.snapshot_index.load(Ordering::Relaxed);
    let purged = recorder.purged_index.load(Ordering::Relaxed);
    let server_state = recorder.server_state.load(Ordering::Relaxed);

    tracing::info!(term, last_log, applied, snapshot, purged, server_state, "gauge metrics");

    assert!(term >= 1, "current_term should be at least 1, got {}", term);
    assert!(last_log > 0, "last_log_index should be > 0, got {}", last_log);
    assert!(applied > 0, "applied_index should be > 0, got {}", applied);
    assert!(snapshot > 0, "snapshot_index should be > 0, got {}", snapshot);
    assert!(purged > 0, "purged_index should be > 0, got {}", purged);
    assert_eq!(
        server_state, 3,
        "server_state should be Leader (3), got {}",
        server_state
    );

    // Counters: should have counted operations
    let heartbeat = recorder.heartbeat_count.load(Ordering::Relaxed);

    tracing::info!(heartbeat, "counter metrics");

    assert!(heartbeat > 0, "heartbeat_count should be > 0, got {}", heartbeat);

    // Note: vote_count and append_count are incremented on followers receiving RPCs,
    // not on the leader. We only installed the recorder on the leader.

    Ok(())
}

/// Test that MetricsRecorder on followers captures append entries and vote operations.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn test_metrics_recorder_on_follower() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    // Install recorder on follower (node 1)
    let recorder = Arc::new(TestRecorder::default());
    let follower = router.get_raft_handle(&1)?;
    follower.set_metrics_recorder(Some(recorder.clone())).await?;

    // Write some entries from leader
    let leader_id = router.leader().expect("cluster should have a leader");
    tracing::info!(log_index, "--- write 5 client requests");
    log_index += router.client_request_many(leader_id, "test", 5).await?;

    // Wait for follower to receive and apply
    router.wait(&1, timeout()).applied_index(Some(log_index), "follower applied").await?;

    // Trigger an election from another node to test vote_count
    // Node 2 will send vote requests to node 1 (which has our recorder)
    tracing::info!("--- trigger election on node 2 to test vote_count");
    let node2 = router.get_raft_handle(&2)?;
    node2.trigger().elect().await?;

    // Wait a bit for vote to be processed
    TypeConfig::sleep(Duration::from_millis(200)).await;

    // Verify follower metrics
    let append_count = recorder.append_count.load(Ordering::Relaxed);
    let append_batch = recorder.append_batch_total.load(Ordering::Relaxed);
    let apply_batch = recorder.apply_batch_total.load(Ordering::Relaxed);
    let vote_count = recorder.vote_count.load(Ordering::Relaxed);
    let term = recorder.current_term.load(Ordering::Relaxed);
    let last_log = recorder.last_log_index.load(Ordering::Relaxed);
    let applied = recorder.applied_index.load(Ordering::Relaxed);
    let server_state = recorder.server_state.load(Ordering::Relaxed);

    tracing::info!(
        append_count,
        append_batch,
        apply_batch,
        vote_count,
        term,
        last_log,
        applied,
        server_state,
        "follower metrics"
    );

    assert!(
        append_count > 0,
        "follower should receive append entries RPCs, got {}",
        append_count
    );
    assert!(
        append_batch > 0,
        "follower should record append batches, got {}",
        append_batch
    );
    assert!(
        apply_batch > 0,
        "follower should record apply batches, got {}",
        apply_batch
    );
    assert!(
        vote_count > 0,
        "follower should receive vote requests, got {}",
        vote_count
    );
    assert!(term >= 1, "current_term should be at least 1, got {}", term);
    assert!(last_log > 0, "last_log_index should be > 0, got {}", last_log);
    assert!(applied > 0, "applied_index should be > 0, got {}", applied);
    // Server state could be Follower(1) or Candidate(2) depending on election timing
    assert!(
        server_state == 1 || server_state == 2,
        "server_state should be Follower(1) or Candidate(2), got {}",
        server_state
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
