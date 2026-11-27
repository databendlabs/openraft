//! A node's per-entry stage latencies and `apply()` batch sizes, from OpenRaft's runtime stats.

use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::storage::RaftStateMachine;

/// Report the stage latencies of the last log ranges this node handled, in µs per entry, and the
/// entries of its `apply()` calls since it started.
///
/// Returns `null` when the node is built without the `runtime-stats` feature.
#[cfg(feature = "runtime-stats")]
pub(crate) async fn report<C, SM>(raft: &Raft<C, SM>) -> Result<serde_json::Value, String>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    use openraft::async_runtime::WatchReceiver;

    let stats = raft.runtime_stats().await.map_err(|error| error.to_string())?;
    let (state, leader) = {
        let metrics = raft.metrics();
        let metrics = metrics.borrow_watched();
        (format!("{:?}", metrics.state), metrics.current_leader.clone())
    };
    let stages = &stats.log_stage_histograms;
    Ok(serde_json::json!({"state":state,"current_leader":leader,
        "log_stages_us":{
            "proposed_to_received":percentiles(&stages.proposed_to_received),
            "received_to_submitted":percentiles(&stages.received_to_submitted),
            "submitted_to_persisted":percentiles(&stages.submitted_to_persisted),
            "persisted_to_committed":percentiles(&stages.persisted_to_committed),
            "committed_to_applied":percentiles(&stages.committed_to_applied),
            "proposed_to_applied":percentiles(&stages.proposed_to_applied)},
        "apply_call_entries":percentiles(&stats.apply_batch)}))
}

#[cfg(not(feature = "runtime-stats"))]
pub(crate) async fn report<C, SM>(_raft: &Raft<C, SM>) -> Result<serde_json::Value, String>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    Ok(serde_json::Value::Null)
}

#[cfg(feature = "runtime-stats")]
fn percentiles(histogram: &openraft::stats::Histogram) -> serde_json::Value {
    let stats = histogram.percentile_stats();
    serde_json::json!({"samples":stats.samples,"p50":stats.p50,"p90":stats.p90,"p99":stats.p99,
        "p99_9":stats.p99_9})
}

#[cfg(all(test, feature = "runtime-stats"))]
mod tests {
    use super::*;

    #[test]
    fn percentiles_report_the_sample_count_and_upper_quantiles() {
        let mut histogram = openraft::stats::Histogram::<()>::new();
        for value in 1..=100 {
            histogram.record(value);
        }

        let report = percentiles(&histogram);

        assert_eq!(100, report["samples"]);
        let p50 = report["p50"].as_u64().unwrap();
        let p99 = report["p99"].as_u64().unwrap();
        assert!((45..=55).contains(&p50), "p50 {p50}");
        assert!(p50 < p99, "p99 {p99}");
    }
}
