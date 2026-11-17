use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use bench_minimal::profile::Profile;
use bench_minimal::profile_value::ProfileValue;
use serde_json::Value;

/// Bounded correctness histories and the latest observed acknowledgement for each key.
pub(super) struct History {
    origin: Instant,
    profile: Profile,
    batch_size: u64,
    sampling: super::arrival::ArrivalConfig,
    state: Mutex<State>,
}

#[derive(Clone, Copy)]
pub(super) struct ReadExpectation {
    pub(super) key: u64,
    version: u64,
    operation: u64,
    invoked: u64,
}

#[derive(Debug)]
pub(super) struct OracleViolation(pub(super) String);

impl std::fmt::Display for OracleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for OracleViolation {}

struct State {
    acknowledged: BTreeMap<u64, (u64, u64)>,
    events: VecDeque<Value>,
    dropped: u64,
    checked_reads: u64,
}

impl History {
    pub(super) fn new(config: &super::BenchConfig) -> Self {
        let profile = config.profile.clone();
        let mut sampling = config.arrival.clone();
        sampling.phase_sample_every = profile.history_every;
        Self {
            origin: Instant::now(),
            profile,
            batch_size: config.batch_size,
            sampling,
            state: Mutex::new(State {
                acknowledged: BTreeMap::new(),
                events: VecDeque::new(),
                dropped: 0,
                checked_reads: 0,
            }),
        }
    }

    pub(super) fn acknowledged(
        &self,
        operation: u64,
        log_id: openraft::alias::LogIdOf<bench_minimal::store::TypeConfig>,
        invoked: Instant,
    ) {
        let version = log_id.index();
        let mut state = self.state.lock().unwrap();
        let completed = self.nanos(Instant::now());
        let key = self.profile.key(operation);
        let previous = state.acknowledged.entry(key).or_insert((0, 0));
        if version > previous.0 {
            *previous = (version, operation);
        }
        if self.sampled(operation, super::arrival::WRITE_STREAM) {
            let event = serde_json::json!({"kind":"write","operation":operation,"key":key,
                "invoked_ns":self.nanos(invoked),"completed_ns":completed,"version":version,"log_id":log_id});
            self.push(&mut state, event);
        }
    }

    pub(super) fn begin_read(&self, operation: u64) -> ReadExpectation {
        let state = self.state.lock().unwrap();
        let key = self.profile.key(operation);
        let (version, operation) = state.acknowledged.get(&key).copied().unwrap_or((0, 0));
        ReadExpectation {
            key,
            version,
            operation,
            invoked: self.nanos(Instant::now()),
        }
    }

    pub(super) fn checked_read(
        &self,
        operation: u64,
        expected: ReadExpectation,
        value: &Option<ProfileValue>,
    ) -> anyhow::Result<()> {
        let ReadExpectation {
            key,
            version: required,
            operation: acknowledged,
            invoked,
        } = expected;
        let version = value.as_ref().map_or(0, |value| value.version);
        let checked = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                version >= required,
                "key {key} read version {version} behind acknowledged {required}"
            );
            if let Some(value) = value.as_ref().filter(|_| self.sampled(operation, super::arrival::READ_STREAM)) {
                anyhow::ensure!(value.request.key == key, "query returned a different key");
                if required > 0 && version == required {
                    anyhow::ensure!(
                        value.request.operation == acknowledged,
                        "query log version belongs to a different operation"
                    );
                }
                let byte = value.request.operation as u8;
                anyhow::ensure!(
                    value.request.value.len() == self.profile.payload_bytes,
                    "query payload length changed"
                );
                anyhow::ensure!(
                    value.request.value.iter().all(|value| *value == byte),
                    "query payload content changed"
                );
            }
            Ok(())
        })();
        checked.map_err(|error| OracleViolation(format!("read operation {operation}: {error:#}")))?;
        let mut state = self.state.lock().unwrap();
        state.checked_reads += 1;
        if self.sampled(operation, super::arrival::READ_STREAM) {
            let event = serde_json::json!({"kind":"read","operation":operation,"key":key,"required_version":required,
                "version":version,"invoked_ns":invoked,"completed_ns":self.nanos(Instant::now())});
            self.push(&mut state, event);
        }
        Ok(())
    }

    pub(super) fn write_outcome(
        &self,
        arrival: super::arrival::Arrival,
        sample: &super::request_sample::RequestSample,
        invoked: Option<Instant>,
    ) {
        if self.sampling.arrival_noop {
            return;
        }
        let Some(invoked) = invoked else {
            return;
        };
        if !self.sampled(arrival.operation, super::arrival::WRITE_STREAM) {
            return;
        }
        let first = arrival.operation * self.batch_size;
        let consumed = sample.write_progress.as_ref().map_or(
            u64::from(sample.outcome == super::request_sample::Outcome::Completed),
            |progress| progress.completed + progress.failed,
        );
        let mut state = self.state.lock().unwrap();
        let event = serde_json::json!({"kind":"write_outcome","first_operation":first,
            "entries":self.batch_size,"responses_consumed":consumed,
            "responses_completed":sample.write_progress.as_ref().map_or(consumed, |progress| progress.completed),
            "responses_failed":sample.write_progress.as_ref().map_or(0, |progress| progress.failed),"unknown_first":first+consumed,
            "unknown_entries":self.batch_size-consumed,"outcome":format!("{:?}",sample.outcome),"retries":sample.retries,
            "scheduled_ns":self.nanos(arrival.scheduled),"invoked_ns":self.nanos(invoked),"completed_ns":self.nanos(Instant::now())});
        self.push(&mut state, event);
    }

    pub(super) fn verify_final(
        &self,
        router: &bench_minimal::network::Router,
        nodes: &std::collections::BTreeSet<u64>,
    ) -> anyhow::Result<Value> {
        let state = self.state.lock().unwrap();
        let mut checked_nodes = Vec::new();
        let mut unavailable_nodes = Vec::new();
        let mut behind_nodes = Vec::new();
        for node in nodes {
            let Some(actual) = router.profile_state(*node) else {
                unavailable_nodes.push(*node);
                continue;
            };
            let applied = actual.last_applied_log.map_or(0, |log| log.index());
            let mut behind = 0;
            for (key, (version, operation)) in &state.acknowledged {
                if applied < *version {
                    behind += 1;
                    continue;
                }
                let found = actual
                    .values
                    .get(key)
                    .ok_or_else(|| anyhow::anyhow!("node {node} acknowledged key {key} missing"))?;
                anyhow::ensure!(
                    found.version >= *version,
                    "node {node} key {key} missed acknowledged version {version}"
                );
                anyhow::ensure!(found.request.key == *key, "final value stored under wrong key");
                if found.version == *version {
                    anyhow::ensure!(
                        found.request.operation == *operation,
                        "final log version belongs to a different operation"
                    );
                }
                let byte = found.request.operation as u8;
                anyhow::ensure!(
                    found.request.value.len() == self.profile.payload_bytes,
                    "final payload length changed"
                );
                anyhow::ensure!(
                    found.request.value.iter().all(|value| *value == byte),
                    "final payload changed"
                );
            }
            if behind > 0 {
                behind_nodes.push((*node, behind));
            } else {
                checked_nodes.push(*node);
            }
        }
        Ok(
            serde_json::json!({"checked_nodes":checked_nodes,"locked_nodes":unavailable_nodes,"behind_nodes":behind_nodes,
            "complete":unavailable_nodes.is_empty() && behind_nodes.is_empty(),"acknowledged_keys":state.acknowledged.len()}),
        )
    }

    pub(super) fn report(&self) -> Value {
        let state = self.state.lock().unwrap();
        serde_json::json!({"scope":"observed_client_acknowledgements_before_query_invocation",
            "checked_reads":state.checked_reads,"retained_events":state.events,"dropped_events":state.dropped,
            "event_capacity":self.profile.history_capacity,"acknowledged_keys":state.acknowledged.len()})
    }

    fn sampled(&self, operation: u64, stream: u64) -> bool {
        self.sampling.sampled(operation, stream)
    }

    fn nanos(&self, at: Instant) -> u64 {
        at.duration_since(self.origin).as_nanos() as u64
    }

    fn push(&self, state: &mut State, event: Value) {
        if self.profile.history_capacity == 0 {
            state.dropped += 1;
            return;
        }
        if state.events.len() == self.profile.history_capacity {
            state.events.pop_front();
            state.dropped += 1;
        }
        state.events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrival::Arrival;
    use crate::arrival::ArrivalConfig;
    use crate::arrival::Pattern;
    use crate::request_sample::Outcome;
    use crate::request_sample::RequestSample;

    fn history() -> History {
        History {
            origin: Instant::now(),
            profile: Profile {
                profile: true,
                profile_keys: 1,
                payload_bytes: 4,
                history_every: 1,
                history_capacity: 2,
                ..Profile::default()
            },
            batch_size: 4,
            sampling: ArrivalConfig {
                open_loop: false,
                arrival_pattern: Pattern::Uniform,
                arrival_seed: 1,
                burst_size: 64,
                queue_capacity: 1,
                request_timeout_ms: 0,
                phase_sample_every: 1,
                arrival_noop: false,
            },
            state: Mutex::new(State {
                acknowledged: BTreeMap::new(),
                events: VecDeque::new(),
                dropped: 0,
                checked_reads: 0,
            }),
        }
    }

    #[test]
    fn acknowledgement_floor_is_captured_at_invocation_and_oracle_errors_are_typed() -> anyhow::Result<()> {
        let history = history();
        let log = openraft::testing::log_id::<bench_minimal::store::TypeConfig>(1, 0, 11);
        let before = history.begin_read(0);
        history.acknowledged(7, log, Instant::now());
        let after = history.begin_read(0);
        history.checked_read(0, before, &None)?;
        let error = history.checked_read(0, after, &None).unwrap_err();
        let typed = error.downcast_ref::<OracleViolation>();
        assert!(typed.is_some());
        let request = history.profile.request(8).payload.unwrap();
        let corrupt = Some(ProfileValue { version: 11, request });
        let error = history.checked_read(0, after, &corrupt).unwrap_err();
        let typed = error.downcast_ref::<OracleViolation>();
        assert!(typed.is_some());
        Ok(())
    }

    #[test]
    fn partial_batches_retain_the_confirmed_prefix_and_skip_uninvoked_noops() -> anyhow::Result<()> {
        let mut history = history();
        let now = Instant::now();
        let arrival = Arrival {
            operation: 1,
            scheduled: now,
            released: Some(now),
        };
        let mut sample = RequestSample::new(now, arrival, false)?;
        sample.outcome = Outcome::TimedOut;
        let bounds = crate::measurement::Bounds {
            start: std::time::Duration::ZERO,
            end: None,
            deadline: None,
        };
        let mut progress = crate::write_progress::WriteProgress::new(now, bounds, 4);
        progress.acknowledge(true);
        progress.acknowledge(false);
        sample.write_progress = Some(Box::new(progress));
        history.write_outcome(arrival, &sample, None);
        let empty = history.state.lock().unwrap().events.is_empty();
        assert!(empty);
        history.write_outcome(arrival, &sample, Some(now));
        let event = history.state.lock().unwrap().events[0].clone();
        assert_eq!(4, event["first_operation"]);
        assert_eq!(1, event["responses_completed"]);
        assert_eq!(1, event["responses_failed"]);
        assert_eq!(6, event["unknown_first"]);
        assert_eq!(2, event["unknown_entries"]);
        history.sampling.arrival_noop = true;
        history.write_outcome(arrival, &sample, Some(now));
        let retained = history.state.lock().unwrap().events.len();
        assert_eq!(1, retained);
        Ok(())
    }
}
