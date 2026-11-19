//! Attribute quiescent snapshot operations with payload-bearing application state.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::ensure;
use bench_minimal::network::BenchRaft;
use bench_minimal::profile_value::ProfileValue;
use bench_minimal::store::SnapshotData;
use bench_minimal::store::StateMachine;
use bench_minimal::store::StateMachineStore;
use bench_minimal::store::TypeConfig;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotOf;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use serde::Serialize;

use super::Args;
use super::WAIT_TIMEOUT;
use super::allocator;
use super::create_cluster;
use super::measurement::Measurement;
use super::measurement::Observation;

const QUIESCENCE_POLL: Duration = Duration::from_millis(1);

#[derive(Serialize)]
struct SnapshotRun {
    repetition: usize,
    build_snapshot: BufferObservation,
    get_current_snapshot: BufferObservation,
    encoded_vec_clone: BufferObservation,
}

#[derive(Serialize)]
struct BufferObservation {
    measurement: Observation,
    encoded_len: usize,
    encoded_capacity: usize,
    live_bytes_after_drop: usize,
    retained_live_bytes_delta: i128,
}

impl BufferObservation {
    fn snapshot(measurement: Observation, snapshot: SnapshotOf<TypeConfig, SnapshotData>) -> Self {
        let data = snapshot.snapshot.get_ref();
        let encoded_len = data.len();
        let encoded_capacity = data.capacity();
        drop(snapshot);
        let live_bytes_after_drop = allocator::live_bytes();
        let retained_live_bytes_delta = live_bytes_after_drop as i128 - measurement.live_bytes_before as i128;
        Self {
            measurement,
            encoded_len,
            encoded_capacity,
            live_bytes_after_drop,
            retained_live_bytes_delta,
        }
    }

    fn buffer(measurement: Observation, data: Vec<u8>) -> Self {
        let encoded_len = data.len();
        let encoded_capacity = data.capacity();
        drop(data);
        let live_bytes_after_drop = allocator::live_bytes();
        let retained_live_bytes_delta = live_bytes_after_drop as i128 - measurement.live_bytes_before as i128;
        Self {
            measurement,
            encoded_len,
            encoded_capacity,
            live_bytes_after_drop,
            retained_live_bytes_delta,
        }
    }
}

pub(super) async fn profile(args: &Args) -> anyhow::Result<()> {
    let (mut machine, expected) = prepare(args).await?;
    let mut runs = Vec::with_capacity(args.snapshots);
    for repetition in 0..args.snapshots {
        let build_snapshot = measure_build(&mut machine, &expected).await?;
        let (get_current_snapshot, encoded_vec_clone) = measure_outgoing(&mut machine, &expected).await?;
        runs.push(SnapshotRun {
            repetition,
            build_snapshot,
            get_current_snapshot,
            encoded_vec_clone,
        });
    }
    let report = report(args, &expected, runs);
    println!("{report}");
    Ok(())
}

async fn prepare(args: &Args) -> anyhow::Result<(Arc<StateMachineStore>, StateMachine)> {
    let (router, leader) = create_cluster(args).await?;
    let (values, acknowledged) = write_setup(args, &leader).await?;
    let boundary = acknowledged.last().copied();
    let boundary = boundary.context("snapshot setup has no acknowledged writes")?;
    let wait = leader.wait(Some(WAIT_TIMEOUT));
    wait.applied_index_at_least(Some(boundary.index()), "snapshot setup drained").await?;
    let machine = leader.with_state_machine(|machine| Box::pin(async move { machine.clone() })).await?;
    leader.try_shutdown().await?;
    drop(leader);
    drop(router);
    wait_quiescent(&machine).await?;
    let state = machine.profile_state().await;
    verify_setup(args, &state, &values, &acknowledged)?;
    Ok((machine, state))
}

async fn write_setup(
    args: &Args,
    leader: &BenchRaft,
) -> anyhow::Result<(BTreeMap<u64, ProfileValue>, Vec<LogIdOf<TypeConfig>>)> {
    let mut values = BTreeMap::new();
    let mut acknowledged = Vec::with_capacity(args.operations);
    for operation in 0..args.operations {
        let operation = u64::try_from(operation)?;
        let request = args.profile.request(operation)?;
        let payload = request.payload.as_ref().context("profile request omitted its payload")?;
        let payload = Arc::clone(payload);
        ensure!(
            payload.operation == operation,
            "profile generated a different logical ID"
        );
        let response = leader.client_write(request).await?;
        let version = response.log_id.index();
        let value = ProfileValue {
            version,
            request: payload,
        };
        values.insert(value.request.key, value);
        acknowledged.push(response.log_id);
    }
    Ok((values, acknowledged))
}

async fn wait_quiescent(machine: &Arc<StateMachineStore>) -> anyhow::Result<()> {
    let stopped = async {
        while Arc::strong_count(machine) != 1 {
            tokio::time::sleep(QUIESCENCE_POLL).await;
        }
    };
    let result = tokio::time::timeout(WAIT_TIMEOUT, stopped).await;
    result.context("state-machine owners did not stop after node shutdown")?;
    Ok(())
}

fn verify_setup(
    args: &Args,
    state: &StateMachine,
    values: &BTreeMap<u64, ProfileValue>,
    acknowledged: &[LogIdOf<TypeConfig>],
) -> anyhow::Result<()> {
    let boundary = acknowledged.last().copied();
    ensure!(
        state.last_applied_log == boundary,
        "setup applied boundary differs from its acknowledgement"
    );
    ensure!(
        state.values == *values,
        "setup values differ from acknowledged payloads"
    );
    let count = u64::try_from(acknowledged.len())?;
    ensure!(state.applied_writes == count, "setup write count changed");
    ensure!(state.duplicates == 0, "setup contains duplicate logical writes");
    let skipped = acknowledged.len().saturating_sub(args.profile.duplicate_capacity);
    let indexed = acknowledged.iter().enumerate();
    let retained = indexed.skip(skipped);
    let seen: HashMap<_, _> = retained.map(|(operation, log)| (operation as u64, log.index())).collect();
    let recent: VecDeque<_> = (skipped as u64..count).collect();
    ensure!(state.seen == seen, "setup retained logical IDs or versions changed");
    ensure!(state.recent == recent, "setup logical-ID retention order changed");
    ensure!(
        state.evicted_ids == skipped as u64,
        "setup logical-ID eviction count changed"
    );
    Ok(())
}

async fn measure_build(
    machine: &mut Arc<StateMachineStore>,
    expected: &StateMachine,
) -> anyhow::Result<BufferObservation> {
    let measurement = Measurement::start_peak();
    let mut builder = machine.get_snapshot_builder().await;
    let snapshot = builder.build_snapshot().await?;
    let observation = measurement.finish();
    drop(builder);
    verify_snapshot(&snapshot, expected)?;
    Ok(BufferObservation::snapshot(observation, snapshot))
}

async fn measure_outgoing(
    machine: &mut Arc<StateMachineStore>,
    expected: &StateMachine,
) -> anyhow::Result<(BufferObservation, BufferObservation)> {
    let measurement = Measurement::start_peak();
    let output = machine.get_current_snapshot().await?;
    let observation = measurement.finish();
    let output = output.context("built snapshot is absent")?;
    verify_snapshot(&output, expected)?;
    let source = output.snapshot.get_ref();
    let measurement = Measurement::start_peak();
    let cloned = Vec::clone(source);
    std::hint::black_box(&cloned);
    let cloned_observation = measurement.finish();
    ensure!(cloned == *source, "encoded-buffer clone changed snapshot bytes");
    let encoded_vec_clone = BufferObservation::buffer(cloned_observation, cloned);
    let get_current_snapshot = BufferObservation::snapshot(observation, output);
    Ok((get_current_snapshot, encoded_vec_clone))
}

fn verify_snapshot(snapshot: &SnapshotOf<TypeConfig, SnapshotData>, expected: &StateMachine) -> anyhow::Result<()> {
    let bytes = snapshot.snapshot.get_ref();
    let actual: StateMachine = serde_json::from_slice(bytes)?;
    ensure!(
        snapshot.meta.last_log_id == expected.last_applied_log,
        "snapshot metadata applied boundary changed"
    );
    ensure!(
        snapshot.meta.last_membership == expected.last_membership,
        "snapshot metadata membership changed"
    );
    ensure!(
        actual.last_applied_log == expected.last_applied_log,
        "snapshot applied boundary changed"
    );
    ensure!(
        actual.last_membership == expected.last_membership,
        "snapshot membership changed"
    );
    ensure!(
        actual.applied_writes == expected.applied_writes,
        "snapshot write count changed"
    );
    ensure!(actual.values == expected.values, "snapshot values changed");
    ensure!(actual.seen == expected.seen, "snapshot retained logical IDs changed");
    ensure!(actual.recent == expected.recent, "snapshot logical-ID order changed");
    ensure!(
        actual.duplicates == expected.duplicates,
        "snapshot duplicate count changed"
    );
    ensure!(
        actual.evicted_ids == expected.evicted_ids,
        "snapshot eviction count changed"
    );
    Ok(())
}

fn state_report(state: &StateMachine) -> serde_json::Value {
    let lengths = state.values.values().map(|value| value.request.value.len());
    let logical_value_bytes: usize = lengths.sum();
    serde_json::json!({
        "applied": state.last_applied_log,
        "applied_writes": state.applied_writes,
        "unique_acknowledged_ids": state.applied_writes,
        "keys": state.values.len(),
        "logical_value_bytes": logical_value_bytes,
        "dedup_retained_ids": state.seen.len(),
        "dedup_recent_ids": state.recent.len(),
        "dedup_evicted_ids": state.evicted_ids,
        "duplicate_logical_ids": state.duplicates,
    })
}

fn report(args: &Args, state: &StateMachine, runs: Vec<SnapshotRun>) -> serde_json::Value {
    serde_json::json!({
        "phase": "snapshot_allocations",
        "config": args,
        "runtime_stats": cfg!(feature = "runtime-stats"),
        "state": state_report(state),
        "scope": {
            "setup": "all writes acknowledged/applied; Never snapshot policy; node shut down; other SM Arc owners released",
            "counters": "requested allocation/reallocation bytes across all threads; consecutive atomic boundary loads; allocator internals and RSS excluded",
            "peak": "phase requested-live high-water reset to before-live; allocator-internal realloc overlap excluded",
            "timing": "wall time; outputs remain alive at the end boundary; validation/reporting/drop occur outside",
            "post_drop_live": "process-global live bytes/delta sampled after semantic validation and output release, including retained validation/runtime allocations",
            "build_snapshot": "get_snapshot_builder plus whole build_snapshot, including cache replacement and metadata",
            "get_current_snapshot": "whole outgoing snapshot operation including metadata; output held through clone control",
            "encoded_vec_clone": "Vec clone only; outgoing buffer/metadata captured outside and included in before-live",
        },
        "runs": runs,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::Parser;

    use super::*;

    #[tokio::test]
    async fn payload_snapshots_preserve_the_drained_state_and_id_window() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "profile",
            "--snapshot-allocations",
            "--profile",
            "--clients",
            "1",
            "--members",
            "1",
            "--operations",
            "16",
            "--profile-keys",
            "4",
            "--payload-bytes",
            "256",
            "--snapshots",
            "2",
            "--duplicate-capacity",
            "8",
        ])?;
        super::super::validate(&args)?;
        let (mut machine, expected) = prepare(&args).await?;
        let owners = Arc::strong_count(&machine);
        assert_eq!(1, owners);
        let retained = expected.seen.keys().copied();
        let retained: BTreeSet<_> = retained.collect();
        let wanted: BTreeSet<_> = (8..16).collect();
        assert_eq!(wanted, retained);
        let expected_json = serde_json::to_value(&expected)?;
        for _ in 0..args.snapshots {
            let built = measure_build(&mut machine, &expected).await?;
            let (outgoing, cloned) = measure_outgoing(&mut machine, &expected).await?;
            assert_eq!(built.encoded_len, outgoing.encoded_len);
            assert_eq!(outgoing.encoded_len, cloned.encoded_len);
            let snapshot = machine.get_current_snapshot().await?.context("snapshot is absent")?;
            let decoded: serde_json::Value = serde_json::from_slice(snapshot.snapshot.get_ref())?;
            assert_eq!(expected_json, decoded);
            assert_eq!(expected.last_applied_log, snapshot.meta.last_log_id);
            assert_eq!(expected.last_membership, snapshot.meta.last_membership);
        }
        Ok(())
    }
}
