use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use futures::TryStreamExt;
use openraft::Config;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::SnapshotPolicy;
use openraft::Vote;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::OneshotReceiverOf;
use openraft::alias::OneshotSenderOf;
use openraft::alias::VoteOf;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::WriteOption;
use openraft::stats::RuntimeStats;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::testing::blank_ent;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

fn log_stage_totals(stats: &RuntimeStats<TypeConfig>) -> [u64; 6] {
    let h = &stats.log_stage_histograms;

    [
        h.proposed_to_received.total(),
        h.received_to_submitted.total(),
        h.submitted_to_persisted.total(),
        h.persisted_to_committed.total(),
        h.committed_to_applied.total(),
        h.proposed_to_applied.total(),
    ]
}

fn subtract(after: [u64; 6], before: [u64; 6]) -> [u64; 6] {
    std::array::from_fn(|i| after[i] - before[i])
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_log_stage_counts_batched_entries_per_entry() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- warm up log-stage stats with one client write");
    router.client_request(0, "warmup", 0).await?;
    log_index += 1;
    router.wait(&0).applied_index(Some(log_index), "warmup write applied").await?;

    let before_runtime_stats = n0.runtime_stats().await?;
    let before = log_stage_totals(&before_runtime_stats);

    tracing::info!(log_index, "--- writing 5 entries with client_write_many()");
    let requests: Vec<ClientRequest> = (0..5).map(|i| ClientRequest::make_request("batched", i)).collect();
    let mut stream = n0.client_write_many(requests, WriteOption::default()).await?;

    let mut result_count = 0;
    while let Some(result) = stream.try_next().await? {
        let _resp = result?;
        result_count += 1;
    }

    assert_eq!(
        5, result_count,
        "client_write_many() should yield one result per input entry"
    );

    log_index += 5;
    router.wait(&0).applied_index(Some(log_index), "batched writes applied").await?;

    let runtime_stats = n0.runtime_stats().await?;
    tracing::info!(
        log_index,
        "--- verify busy time and its histogram after processing writes"
    );
    {
        assert!(runtime_stats.core_busy_duration >= before_runtime_stats.core_busy_duration);
        let before_samples = before_runtime_stats.core_busy_histogram.total();
        let after_samples = runtime_stats.core_busy_histogram.total();
        assert!(after_samples > before_samples);
        #[cfg(not(feature = "rt-sim"))]
        {
            assert!(runtime_stats.core_busy_duration > std::time::Duration::ZERO);
            let longest = runtime_stats.core_busy_histogram.percentile(1.0);
            assert!(longest > 0);
        }
    }

    println!("{}", runtime_stats.log_stage);

    let after = log_stage_totals(&runtime_stats);
    let delta = subtract(after, before);

    assert_eq!(
        [5; 6], delta,
        "log stage histograms should count one sample per written entry even when entries are batched"
    );

    assert_eq!(
        before_runtime_stats.write_batch.total() + 1,
        runtime_stats.write_batch.total(),
        "client_write_many() should add exactly one write batch sample"
    );
    assert_eq!(
        before_runtime_stats.append_batch.total() + 1,
        runtime_stats.append_batch.total(),
        "client_write_many() should append the 5 entries in one storage batch"
    );
    assert_eq!(
        before_runtime_stats.apply_batch.total() + 1,
        runtime_stats.apply_batch.total(),
        "client_write_many() should apply the 5 entries in one state machine batch"
    );

    let write_batch = runtime_stats.write_batch.percentile_stats();
    let append_batch = runtime_stats.append_batch.percentile_stats();
    let apply_batch = runtime_stats.apply_batch.percentile_stats();

    assert!(
        write_batch.p99 >= 5,
        "write batch p99 should reflect the 5-entry client_write_many() batch, got {:?}",
        write_batch
    );
    assert!(
        append_batch.p99 >= 5,
        "append batch p99 should reflect the 5-entry storage append, got {:?}",
        append_batch
    );
    assert!(
        apply_batch.p99 >= 5,
        "apply batch p99 should reflect the 5-entry state machine apply, got {:?}",
        apply_batch
    );

    let log_stage_histograms = &runtime_stats.log_stage_histograms;
    assert_eq!(after[0], log_stage_histograms.proposed_to_received.total());
    assert_eq!(after[1], log_stage_histograms.received_to_submitted.total());
    assert_eq!(after[2], log_stage_histograms.submitted_to_persisted.total());
    assert_eq!(after[3], log_stage_histograms.persisted_to_committed.total());
    assert_eq!(after[4], log_stage_histograms.committed_to_applied.total());
    assert_eq!(after[5], log_stage_histograms.proposed_to_applied.total());

    println!("{}", runtime_stats.display());

    Ok(())
}

/// A display of runtime statistics gives the same text after the simulated runtime that created
/// it has exited, because formatting it reads no clock: the clock of `openraft-rt-sim` exists only
/// inside `SimRuntime::block_on`.
///
/// It is a plain test because it must run the cluster in a `block_on` call of its own, and format
/// after that call returns.
#[cfg(feature = "rt-sim")]
#[test]
fn runtime_stats_display_formats_after_the_sim_runtime_exits() -> Result<()> {
    use openraft::SnapshotPolicy;
    use openraft::async_runtime::AsyncRuntime;
    use openraft_rt_sim::SimRuntime;

    let mut runtime = SimRuntime::with_seed(0);
    let (stats, multiline, inside, compact, human) = runtime.block_on(async {
        let config = Config {
            enable_tick: false,
            enable_elect: false,
            enable_heartbeat: false,
            snapshot_policy: SnapshotPolicy::Never,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;
        let log_index = router.new_cluster([0], []).await?;
        router.wait(&0).applied_index(Some(log_index), "bootstrap applied").await?;

        let raft = router.get_raft_handle(&0)?;
        let stats = raft.runtime_stats().await?;
        let segments = stats.log_stage.segments().count();
        assert!(segments > 0, "the bootstrap entries leave log-stage segments");

        let multiline = stats.display().multiline();
        let inside = multiline.to_string();
        let compact = stats.display().compact().to_string();
        let human = stats.display().human_readable().to_string();
        raft.shutdown().await?;
        anyhow::Ok((stats, multiline, inside, compact, human))
    })?;

    assert!(inside.contains("log_stage_segments (T: proposal of the first segment):"));
    assert_eq!(
        inside,
        multiline.to_string(),
        "the display made inside formats the same outside"
    );
    assert_eq!(
        inside,
        stats.display().multiline().to_string(),
        "a display made outside, too"
    );
    assert_eq!(compact, stats.display().compact().to_string());
    assert_eq!(human, stats.display().human_readable().to_string());

    Ok(())
}

/// `process_raft_msg()` records one budget-utilization sample per drain of the API channel, as it
/// records one budget sample, also for a drain that processes several messages or none.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_record_one_raft_msg_usage_sample_per_drain() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- send 10 API messages 10 ms apart, so that the drains end between them"
    );
    let (before, after) = {
        let before = n0.runtime_stats().await?;
        for _ in 0..10 {
            TypeConfig::sleep(Duration::from_millis(10)).await;
            n0.with_raft_state(|_| ()).await?;
        }
        TypeConfig::sleep(Duration::from_millis(10)).await;
        let after = n0.runtime_stats().await?;
        (before, after)
    };

    tracing::info!(
        log_index,
        "--- every drain adds one budget sample and one utilization sample"
    );
    {
        let drains = after.raft_msg_budget.total() - before.raft_msg_budget.total();
        let samples = after.raft_msg_usage_permille.total() - before.raft_msg_usage_permille.total();
        assert!(
            drains >= 1,
            "the messages are processed in drains after the first snapshot"
        );
        assert_eq!(drains, samples, "one utilization sample per drain");
    }

    Ok(())
}

/// The request of the leader of `term`, node 0, to append blank logs of `term` at `indexes` after
/// `prev`, which commits them if `commit`.
fn append_request(
    term: u64,
    prev: Option<LogIdOf<TypeConfig>>,
    indexes: std::ops::Range<u64>,
    commit: bool,
) -> AppendEntriesRequest<TypeConfig> {
    let last = indexes.clone().last().map(|index| log_id(term, 0, index));
    AppendEntriesRequest {
        vote: Vote::new_committed(term, 0),
        prev_log_id: prev,
        entries: indexes.map(|index| blank_ent::<TypeConfig>(term, 0, index)).collect(),
        leader_commit: if commit { last } else { None },
    }
}

/// Append a blank log of `term` at `index` after `prev`, and commit it if `commit`.
async fn append_blank(
    raft: &MemRaft,
    term: u64,
    prev: Option<LogIdOf<TypeConfig>>,
    index: u64,
    commit: bool,
) -> Result<()> {
    let request = append_request(term, prev, index..index + 1, commit);
    let reply = raft.append_entries(request).await?;
    assert_eq!(Ok(Some(log_id(term, 0, index))), reply);
    Ok(())
}

/// A follower that replaces an unapplied log observes the replacement afresh: the proposal and
/// receipt it backfills are the replacement's submission, not the discarded log's.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_replaced_log_is_observed_afresh() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        enable_heartbeat: false,
        snapshot_policy: SnapshotPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(1).await?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!("--- append log 0 of term 1, then replace it with a committed log 0 of term 2");
    {
        append_blank(&n1, 1, None, 0, false).await?;
        append_blank(&n1, 2, None, 0, true).await?;
        router.wait(&1).applied_index(Some(0), "the replacement is applied").await?;
    }

    tracing::info!("--- one segment, 0..1, whose proposal and receipt are its submission");
    {
        let stats = n1.runtime_stats().await?;
        let segments: Vec<_> = stats.log_stage.segments().collect();
        assert_eq!(1, segments.len(), "segments: {}", stats.log_stage);
        assert_eq!(0..1, segments[0].range);

        // The values are in stage order: proposed, received, submitted, persisted, committed,
        // applied.
        let values = segments[0].values;
        assert_eq!(values[2], values[0], "proposed is the replacement's submission");
        assert_eq!(values[2], values[1], "received is the replacement's submission");
    }

    Ok(())
}

/// A follower whose log-stage capacity holds two batches discards three unapplied logs and applies
/// a replacement at index 0: the evicted history does not hide the replacement, which adds one
/// sample to each log-stage histogram.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_replacement_before_the_retained_history_is_observed() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        enable_heartbeat: false,
        snapshot_policy: SnapshotPolicy::Never,
        log_stage_capacity: Some(2),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(1).await?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!("--- append logs 0, 1 and 2 of term 1 one at a time: the first batch is evicted");
    {
        for index in 0..3u64 {
            let prev = index.checked_sub(1).map(|prev| log_id(1, 0, prev));
            append_blank(&n1, 1, prev, index, false).await?;
        }
    }

    tracing::info!("--- replace them with a committed log 0 of term 2");
    {
        append_blank(&n1, 2, None, 0, true).await?;
        router.wait(&1).applied_index(Some(0), "the replacement is applied").await?;
    }

    tracing::info!("--- the replacement is the one segment, with one sample in each histogram");
    {
        let stats = n1.runtime_stats().await?;
        let ranges: Vec<_> = stats.log_stage.segments().map(|seg| seg.range).collect();
        assert_eq!(vec![0..1], ranges, "segments: {}", stats.log_stage);
        assert_eq!([1; 6], log_stage_totals(&stats));
    }

    Ok(())
}

/// A log store that hands the flush callbacks of its first `held` appends to the test, in order,
/// and flushes the later ones at once.
///
/// A held append is readable at once, as `RaftLogStorage::append()` requires.
struct HeldFlushLogStore {
    inner: MemLogStore,
    flushes: Arc<Mutex<VecDeque<OneshotSenderOf<TypeConfig, IOFlushed<TypeConfig>>>>>,
}

impl HeldFlushLogStore {
    fn new(inner: MemLogStore, held: usize) -> (Self, Vec<OneshotReceiverOf<TypeConfig, IOFlushed<TypeConfig>>>) {
        let (senders, receivers): (VecDeque<_>, Vec<_>) = (0..held).map(|_| TypeConfig::oneshot()).unzip();
        let store = Self {
            inner,
            flushes: Arc::new(Mutex::new(senders)),
        };
        (store, receivers)
    }
}

impl RaftLogStorage<TypeConfig> for HeldFlushLogStore {
    type LogReader = MemLogStore;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        self.inner.get_log_state().await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.inner.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<TypeConfig>) -> Result<(), io::Error> {
        self.inner.save_vote(vote).await
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let held = self.flushes.lock().unwrap().pop_front();
        let Some(flush) = held else {
            return self.inner.append(entries, callback).await;
        };
        self.inner.append(entries, IOFlushed::noop()).await?;
        let sent = flush.send(callback);
        assert!(sent.is_ok(), "the test keeps the flush receiver");
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<TypeConfig>>) -> Result<(), io::Error> {
        self.inner.truncate_after(last_log_id).await
    }

    async fn purge(&mut self, log_id: LogIdOf<TypeConfig>) -> Result<(), io::Error> {
        self.inner.purge(log_id).await
    }
}

/// A flush of logs that a truncation discarded, which completes after the truncation, records no
/// stage: a later log at a discarded index is observed from its own submission.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_late_flush_of_discarded_logs_records_nothing() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_elect: false,
            enable_heartbeat: false,
            snapshot_policy: SnapshotPolicy::Never,
            ..Default::default()
        }
        .validate()?,
    );
    let router = RaftRouter::new(config.as_ref().clone())?;
    let (log_store, sm) = openraft_memstore::new_mem_store();
    let (log_store, held) = HeldFlushLogStore::new(log_store, 2);
    let mut held = held.into_iter();
    let n1 = Raft::new(1, config, router.clone(), log_store, sm).await?;

    tracing::info!("--- append logs 0..3 of term 1, and hold their flush");
    let (discarded, discarded_flush) = {
        let n1 = n1.clone();
        let request = append_request(1, None, 0..3, false);
        let append = TypeConfig::spawn(async move { n1.append_entries(request).await });
        let flush = TypeConfig::timeout(WAIT_TIMEOUT, held.next().unwrap()).await??;
        (append, flush)
    };

    tracing::info!("--- replace them with a committed log 0 of term 2, and hold its flush too");
    let (replacing, replacing_flush) = {
        let n1 = n1.clone();
        let request = append_request(2, None, 0..1, true);
        let append = TypeConfig::spawn(async move { n1.append_entries(request).await });
        let flush = TypeConfig::timeout(WAIT_TIMEOUT, held.next().unwrap()).await??;
        (append, flush)
    };

    tracing::info!("--- the discarded logs' flush completes first, then the replacement's");
    {
        discarded_flush.io_completed(Ok(()));
        let reply = TypeConfig::timeout(WAIT_TIMEOUT, discarded).await??;
        tracing::info!("the discarded append replies: {:?}", reply);

        replacing_flush.io_completed(Ok(()));
        let reply = TypeConfig::timeout(WAIT_TIMEOUT, replacing).await???;
        assert_eq!(Ok(Some(log_id(2, 0, 0))), reply);
        n1.wait(timeout()).applied_index(Some(0), "the replacement is applied").await?;
    }

    tracing::info!("--- append a committed log 1 of term 2: its proposal and receipt are its submission");
    {
        let reply = n1.append_entries(append_request(2, Some(log_id(2, 0, 0)), 1..2, true)).await?;
        assert_eq!(Ok(Some(log_id(2, 0, 1))), reply);
        n1.wait(timeout()).applied_index(Some(1), "log 1 is applied").await?;

        let stats = n1.runtime_stats().await?;
        let segment = stats.log_stage.segments().find(|seg| seg.range.contains(&1));
        let segment = segment.expect("a segment covers log 1");
        let values = segment.values;
        assert_eq!(
            values[2], values[0],
            "proposed is log 1's submission: {}",
            stats.log_stage
        );
        assert_eq!(
            values[2], values[1],
            "received is log 1's submission: {}",
            stats.log_stage
        );
    }

    n1.shutdown().await?;
    Ok(())
}

/// The log-stage segment ranges.
fn log_stage_ranges(stats: &RuntimeStats<TypeConfig>) -> Vec<std::ops::Range<u64>> {
    stats.log_stage.segments().map(|seg| seg.range).collect()
}

/// A restarted node observes only the logs after the ones its state machine already applied: the
/// new leader's blank log is the one segment, with one sample in each histogram.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_restart_observes_only_the_logs_after_the_applied_ones() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        enable_heartbeat: false,
        snapshot_policy: SnapshotPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(
        log_index,
        "--- apply 5 writes, then restart node 0 with the same stores"
    );
    {
        let written = router.client_request_many(0, "before", 5).await?;
        log_index += written.count;
        router.wait(&0).applied_index(Some(log_index), "the writes are applied").await?;

        let node = router.stop_node(0).await?;
        router.new_raft_node_with_sto(0, node.log_store, node.state_machine).await?;
        log_index += 1;
        router.wait(&0).applied_index(Some(log_index), "the new leader's blank log is applied").await?;
    }

    tracing::info!(log_index, "--- only the blank log is observed");
    {
        let stats = router.get_raft_handle(&0)?.runtime_stats().await?;
        assert_eq!(
            vec![log_index..log_index + 1],
            log_stage_ranges(&stats),
            "segments: {}",
            stats.log_stage
        );
        assert_eq!([1; 6], log_stage_totals(&stats));
    }

    Ok(())
}

/// A node that starts with an empty state machine re-applies the committed logs before its core
/// runs, and does not count them. The logs stored after them, which it commits after starting,
/// are observed together with the new leader's blank log.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_startup_replay_is_not_observed_but_a_later_suffix_is() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        enable_heartbeat: false,
        snapshot_policy: SnapshotPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- apply 5 writes and stop node 0");
    let mut log_store = {
        let written = router.client_request_many(0, "before", 5).await?;
        log_index += written.count;
        router.wait(&0).applied_index(Some(log_index), "the writes are applied").await?;
        router.stop_node(0).await?.log_store
    };
    let applied_end = log_index + 1;

    tracing::info!(
        log_index,
        "--- store 2 uncommitted logs, then restart with an empty state machine"
    );
    {
        let suffix = (applied_end..applied_end + 2).map(|index| blank_ent::<TypeConfig>(1, 0, index));
        log_store.append(suffix.collect::<Vec<_>>(), IOFlushed::noop()).await?;
        log_index += 2;

        let (_, state_machine) = openraft_memstore::new_mem_store();
        router.new_raft_node_with_sto(0, log_store, state_machine).await?;
        log_index += 1;
        router.wait(&0).applied_index(Some(log_index), "the suffix and the blank log are applied").await?;
    }

    tracing::info!(
        log_index,
        "--- the re-applied logs are not observed; the suffix and the blank log are"
    );
    {
        let stats = router.get_raft_handle(&0)?.runtime_stats().await?;
        assert_eq!(
            vec![applied_end..log_index + 1],
            log_stage_ranges(&stats),
            "segments: {}",
            stats.log_stage
        );
        assert_eq!([3; 6], log_stage_totals(&stats));
    }

    Ok(())
}

/// A follower that installs a snapshot does not observe the logs the snapshot covers: the log it
/// appends after the snapshot is the one segment.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_stats_logs_covered_by_an_installed_snapshot_are_not_observed() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        enable_heartbeat: false,
        snapshot_policy: SnapshotPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- apply 5 writes on node 0 and build a snapshot of them");
    let snapshot = {
        let written = router.client_request_many(0, "snapshot", 5).await?;
        log_index += written.count;
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "the snapshot is built").await?;
        n0.get_snapshot().await?.expect("node 0 has a snapshot")
    };

    tracing::info!(
        log_index,
        "--- an empty node 1 installs the snapshot, then appends the next log"
    );
    let n1 = {
        router.new_raft_node(1).await?;
        let n1 = router.get_raft_handle(&1)?;
        n1.install_full_snapshot(Vote::new_committed(1, 0), snapshot).await?;

        let prev = Some(log_id(1, 0, log_index));
        let reply = n1.append_entries(append_request(1, prev, log_index + 1..log_index + 2, true)).await?;
        log_index += 1;
        assert_eq!(Ok(Some(log_id(1, 0, log_index))), reply);
        router.wait(&1).applied_index(Some(log_index), "the log after the snapshot is applied").await?;
        n1
    };

    tracing::info!(log_index, "--- only the log after the snapshot is observed");
    {
        let stats = n1.runtime_stats().await?;
        assert_eq!(
            vec![log_index..log_index + 1],
            log_stage_ranges(&stats),
            "segments: {}",
            stats.log_stage
        );
        assert_eq!([1; 6], log_stage_totals(&stats));
    }

    Ok(())
}
