use std::sync::Arc;

use anyhow::Result;
use futures::TryStreamExt;
use maplit::btreeset;
use openraft::Config;
use openraft::stats::RuntimeStats;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

fn log_stage_totals(stats: &RuntimeStats<TypeConfig>) -> [u64; 6] {
    let h = &stats.log_stage_histograms;

    [
        h.proposed_to_received.total(),
        h.received_to_appended.total(),
        h.appended_to_persisted.total(),
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
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config);

    tracing::info!("--- initializing single-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- warm up log-stage stats with one client write");
    router.client_request(0, "warmup", 0).await?;
    log_index += 1;
    router.wait(&0, None).applied_index(Some(log_index), "warmup write applied").await?;

    let before_runtime_stats = n0.runtime_stats().await?;
    let before = log_stage_totals(&before_runtime_stats);

    tracing::info!(log_index, "--- writing 5 entries with client_write_many()");
    let requests: Vec<ClientRequest> = (0..5).map(|i| ClientRequest::make_request("batched", i)).collect();
    let mut stream = n0.client_write_many(requests).await?;

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
    router.wait(&0, None).applied_index(Some(log_index), "batched writes applied").await?;

    let runtime_stats = n0.runtime_stats().await?;
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
    assert_eq!(after[1], log_stage_histograms.received_to_appended.total());
    assert_eq!(after[2], log_stage_histograms.appended_to_persisted.total());
    assert_eq!(after[3], log_stage_histograms.persisted_to_committed.total());
    assert_eq!(after[4], log_stage_histograms.committed_to_applied.total());
    assert_eq!(after[5], log_stage_histograms.proposed_to_applied.total());

    Ok(())
}
