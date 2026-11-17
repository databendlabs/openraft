//! Application-profile implementation, separate from the empty store hot path.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use futures::Stream;
use futures::TryStreamExt;
use openraft::OptionalSend;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::impls::EntryPayload;
use openraft::storage::EntryResponder;

use super::ClientResponse;
use super::LogStore;
use super::StateMachine;
use super::StateMachineStore;
use super::TypeConfig;
use super::blocking_delay;
use crate::profile::Profile;
use crate::profile::transfer_time;
use crate::profile_value::ProfileValue;

const DELAY_SAMPLES: usize = 4096;

#[derive(Default)]
pub(super) struct DelayStats {
    count: u64,
    samples: VecDeque<(u64, u64)>,
}

impl DelayStats {
    fn record(&mut self, requested: Duration, elapsed: Duration) {
        self.count += 1;
        if self.samples.len() == DELAY_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back((requested.as_nanos() as u64, elapsed.as_nanos() as u64));
    }

    fn report(&self) -> serde_json::Value {
        let mut elapsed: Vec<_> = self.samples.iter().map(|(_, elapsed)| *elapsed).collect();
        elapsed.sort_unstable();
        let percentile =
            |percent: usize| elapsed.get((elapsed.len() * percent).div_ceil(100).saturating_sub(1)).copied();
        serde_json::json!({"samples":self.count,"retained":elapsed.len(),
            "elapsed_p50_ns":percentile(50),"elapsed_p99_ns":percentile(99),
            "requested_and_elapsed_ns":self.samples,"scope":"recent_bounded_observations"})
    }
}

#[derive(Default)]
pub(super) struct ReadStats {
    hits: u64,
    misses: u64,
    bytes: u64,
    miss_bytes: u64,
    delay: DelayStats,
}

impl LogStore {
    /// Enable recent-tail cache and byte-cost observations on this node.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn with_profile(mut self, profile: Arc<Profile>, node: u64) -> Self {
        self.profile = Some(profile);
        self.node = node;
        self
    }

    pub(super) async fn profiled_read<RB>(&self, range: RB, fixed: Duration) -> io::Result<Vec<EntryOf<TypeConfig>>>
    where RB: std::ops::RangeBounds<u64> + Clone {
        let profile = self.profile.as_ref().expect("profile enabled");
        let log = self.log.read().await;
        let tail = log.last_key_value().map_or(0, |(index, _)| index.saturating_add(1));
        let cache_start = tail.saturating_sub(profile.cache_entries);
        let entries: Vec<_> = log.range(range).map(|(_, entry)| entry.clone()).collect();
        drop(log);
        let missed = entries.iter().any(|entry| entry.index() < cache_start);
        let bytes: usize = entries.iter().map(entry_bytes).sum();
        let miss_bytes: usize = entries.iter().filter(|entry| entry.index() < cache_start).map(entry_bytes).sum();
        let miss = if missed {
            Duration::from_micros(profile.cache_miss_us)
        } else {
            Duration::ZERO
        };
        let transfer = transfer_time(miss_bytes, profile.log_read_bytes_per_second);
        let fixed = fixed + miss + profile.node_delay(self.node);
        let delay = fixed + transfer;
        let started = Instant::now();
        blocking_delay(fixed).await?;
        self.read_bandwidth.wait(transfer, blocking_delay).await?;
        let mut reads = self.reads.lock().unwrap();
        reads.miss_bytes += miss_bytes as u64;
        reads.record(missed, bytes, delay, started.elapsed());
        Ok(entries)
    }

    /// Report modeled cache decisions and actual log-read wait distributions.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn profile_report(&self) -> serde_json::Value {
        let reads = self.reads.lock().unwrap();
        let count = reads.hits + reads.misses;
        let rate = if count == 0 {
            None
        } else {
            Some(reads.hits as f64 / count as f64)
        };
        serde_json::json!({"cache_hits":reads.hits,"cache_misses":reads.misses,"all_cached_read_fraction":rate,
            "logical_read_bytes":reads.bytes,"miss_read_bytes":reads.miss_bytes,"bandwidth_scope":"uncached_bytes_shared_by_node",
            "delay":reads.delay.report()})
    }
}

impl ReadStats {
    fn record(&mut self, missed: bool, bytes: usize, requested: Duration, elapsed: Duration) {
        if missed {
            self.misses += 1;
        } else {
            self.hits += 1;
        }
        self.bytes += bytes as u64;
        self.delay.record(requested, elapsed);
    }
}

fn entry_bytes(entry: &EntryOf<TypeConfig>) -> usize {
    let bytes = match &entry.payload {
        EntryPayload::Normal(request) => request.payload.as_ref().map_or(0, |data| data.value.len()),
        _ => 0,
    };
    size_of::<EntryOf<TypeConfig>>() + bytes
}

impl StateMachineStore {
    /// Enable modeled application work on this node.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn with_profile(mut self, profile: Arc<Profile>, node: u64) -> Self {
        self.profile = Some(profile);
        self.node = node;
        self
    }

    /// Read one value and its applied boundary under the same publication lock.
    #[openraft::macros::since(version = "0.10.1")]
    pub async fn query(&self, key: u64) -> (Option<super::LogIdOf<TypeConfig>>, Option<ProfileValue>, Instant) {
        let state = self.sm.read().await;
        let locked = Instant::now();
        let value = state.values.get(&key).cloned();
        (state.last_applied_log, value, locked)
    }

    /// Copy application state for bounded post-workload verification and snapshots.
    #[openraft::macros::since(version = "0.10.1")]
    pub async fn profile_state(&self) -> StateMachine {
        self.sm.read().await.clone()
    }

    /// Inspect published profile state without waiting behind an unfinished transaction.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn try_profile_state(&self) -> Option<StateMachine> {
        let state = self.sm.try_read().ok()?;
        Some(state.clone())
    }

    /// Report actual transaction wait observations; CPU work is performed separately.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn profile_report(&self) -> serde_json::Value {
        self.transactions.lock().unwrap().report()
    }

    pub(super) async fn apply_profile<Strm>(&self, entries: Strm) -> io::Result<()>
    where Strm: Stream<Item = io::Result<EntryResponder<TypeConfig>>> + Unpin + OptionalSend {
        let profile = self.profile.as_ref().expect("profile enabled");
        let mut locked = if profile.concurrent_queries {
            None
        } else {
            Some(self.sm.write().await)
        };
        let entries: Vec<_> = entries.try_collect().await?;
        let bytes: usize = entries.iter().map(|(entry, _)| entry_bytes(entry)).sum();
        for (entry, _) in &entries {
            let EntryPayload::Normal(request) = &entry.payload else {
                continue;
            };
            let Some(data) = &request.payload else { continue };
            data.work(profile.cpu_rounds);
        }
        let count = u32::try_from(entries.len()).map_err(io::Error::other)?;
        let delay = self.apply_io_delay + self.apply_entry_delay * count;
        let delay = delay + transfer_time(bytes, profile.apply_bytes_per_second) + profile.node_delay(self.node);
        let started = Instant::now();
        blocking_delay(delay).await?;
        self.transactions.lock().unwrap().record(delay, started.elapsed());
        #[cfg(test)]
        {
            let gate = self.publication_gate.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.await.map_err(io::Error::other)?;
            }
        }
        if locked.is_none() {
            locked = Some(self.sm.write().await);
        }
        let state = locked.as_mut().expect("publication lock acquired");
        let mut responders = Vec::new();
        for (entry, responder) in entries {
            state.apply_profile_entry(entry, profile.duplicate_capacity);
            if let Some(responder) = responder {
                responders.push(responder);
            }
        }
        let applied = state.last_applied_log;
        drop(locked);
        for responder in responders {
            responder.send(ClientResponse {});
        }
        self.notify_applied(applied, count);
        Ok(())
    }
}

impl StateMachine {
    pub(super) fn trim_profile_ids(&mut self, capacity: usize) {
        while self.recent.len() > capacity {
            let oldest = self.recent.pop_front().expect("oversized ID window");
            self.seen.remove(&oldest);
            self.evicted_ids += 1;
        }
    }

    fn apply_profile_entry(&mut self, entry: EntryOf<TypeConfig>, capacity: usize) {
        self.last_applied_log = Some(entry.log_id);
        match entry.payload {
            EntryPayload::Blank => {}
            EntryPayload::Membership(mem) => {
                self.last_membership = super::StoredMembershipOf::<TypeConfig>::new(Some(entry.log_id), mem);
            }
            EntryPayload::Normal(request) => {
                self.applied_writes += 1;
                if let Some(request) = request.payload {
                    self.put(request, entry.log_id.index(), capacity);
                }
            }
        }
    }

    fn put(&mut self, request: Arc<crate::profile_request::ProfileRequest>, version: u64, capacity: usize) {
        if self.seen.contains_key(&request.operation) {
            self.duplicates += 1;
            self.values.insert(request.key, ProfileValue { version, request });
            return;
        }
        while self.recent.len() >= capacity {
            let oldest = self.recent.pop_front().expect("full ID window");
            self.seen.remove(&oldest);
            self.evicted_ids += 1;
        }
        self.recent.push_back(request.operation);
        self.seen.insert(request.operation, version);
        self.values.insert(request.key, ProfileValue { version, request });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::FutureExt;
    use futures::stream;
    use openraft::RaftTypeConfig;
    use openraft::alias::CommittedLeaderIdOf;
    use openraft::alias::EntryOf;
    use openraft::entry::RaftEntry;
    use openraft::impls::EntryPayload;
    use openraft::storage::RaftSnapshotBuilder;
    use openraft::storage::RaftStateMachine;

    use super::super::ClientRequest;
    use super::super::StateMachineStore;
    use super::super::TypeConfig;
    use crate::profile::Profile;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct EmptyRequest {}

    impl std::fmt::Display for EmptyRequest {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("empty")
        }
    }

    #[test]
    fn optional_payload_fits_the_existing_entry_and_batch_layout() {
        type EmptyPayload = EntryPayload<EmptyRequest, <TypeConfig as RaftTypeConfig>::Membership>;
        type EmptyEntry = openraft::impls::Entry<CommittedLeaderIdOf<TypeConfig>, EmptyPayload>;
        let original_entry = size_of::<EmptyEntry>();
        let profiled_entry = size_of::<EntryOf<TypeConfig>>();
        assert_eq!(original_entry, profiled_entry);
        let original_batch = size_of::<openraft::impls::InlineBatch<EmptyPayload>>();
        let profiled_batch = size_of::<openraft::impls::InlineBatch<<TypeConfig as RaftTypeConfig>::Payload>>();
        assert_eq!(original_batch, profiled_batch);
        let request = ClientRequest::default();
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!("{}", encoded);
    }

    #[tokio::test]
    async fn staged_publication_keeps_queries_available_and_snapshots_coherent() -> anyhow::Result<()> {
        let profile = Profile {
            profile: true,
            concurrent_queries: true,
            payload_bytes: 256,
            ..Profile::default()
        };
        let mut machine = Arc::new(
            StateMachineStore::new()
                .with_profile(Arc::new(profile.clone()), 0)
                .with_apply_io_delay(Duration::from_millis(20)),
        );
        let log = openraft::testing::log_id::<TypeConfig>(1, 0, 1);
        let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(profile.request(7)));
        let entries = stream::iter([Ok((entry, None))]);
        let observing = machine.clone();
        let (release, gate) = tokio::sync::oneshot::channel();
        *machine.publication_gate.lock().unwrap() = Some(gate);
        let mut applying = Box::pin(machine.apply(entries));
        let polled = applying.as_mut().now_or_never();
        assert!(polled.is_none());
        let state = observing.sm.try_read()?;
        assert_eq!(None, state.last_applied_log);
        assert!(state.values.is_empty());
        drop(state);
        release.send(()).unwrap();
        applying.await?;
        let expected = machine.profile_state().await;
        let value = expected.values.get(&profile.key(7)).unwrap();
        assert_eq!(1, value.version);
        assert_eq!(7, value.request.operation);
        let snapshot = machine.build_snapshot().await?;
        let mut restored = Arc::new(StateMachineStore::new().with_profile(Arc::new(profile), 1));
        restored.install_snapshot(&snapshot.meta, snapshot.snapshot).await?;
        let actual = restored.profile_state().await;
        assert_eq!(expected.values, actual.values);
        assert_eq!(expected.last_applied_log, actual.last_applied_log);
        assert_eq!(expected.seen, actual.seen);
        Ok(())
    }

    #[tokio::test]
    async fn duplicates_are_detected_in_a_bounded_window() -> anyhow::Result<()> {
        let profile = Profile {
            profile: true,
            duplicate_capacity: 2,
            ..Profile::default()
        };
        let mut machine = Arc::new(StateMachineStore::new().with_profile(Arc::new(profile.clone()), 0));
        for (index, operation) in [(1, 7), (2, 7), (3, 8), (4, 9)] {
            let log = openraft::testing::log_id::<TypeConfig>(1, 0, index);
            let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(profile.request(operation)));
            machine.apply(stream::iter([Ok((entry, None))])).await?;
        }
        let state = machine.profile_state().await;
        assert_eq!(1, state.duplicates);
        assert_eq!(1, state.evicted_ids);
        let actual: Vec<_> = state.recent.into_iter().collect();
        assert_eq!(vec![8, 9], actual);
        let mut actual: Vec<_> = state.seen.keys().copied().collect();
        actual.sort_unstable();
        assert_eq!(vec![8, 9], actual);
        Ok(())
    }
}

#[cfg(test)]
mod budget_tests {
    use openraft::storage::RaftSnapshotBuilder;
    use openraft::storage::RaftStateMachine;

    use super::*;

    #[tokio::test]
    async fn snapshot_restore_trims_the_target_id_window() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            duplicate_capacity: 8,
            ..Profile::default()
        });
        let mut source = Arc::new(StateMachineStore::new().with_profile(profile.clone(), 0));
        for index in 1..=4 {
            let log = openraft::testing::log_id::<TypeConfig>(1, 0, index);
            let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(profile.request(index)));
            source.apply(futures::stream::iter([Ok((entry, None))])).await?;
        }
        let snapshot = source.build_snapshot().await?;
        let target_profile = Profile {
            duplicate_capacity: 2,
            ..(*profile).clone()
        };
        let mut target = Arc::new(StateMachineStore::new().with_profile(Arc::new(target_profile), 1));
        target.install_snapshot(&snapshot.meta, snapshot.snapshot).await?;
        let restored = target.profile_state().await;
        let actual: Vec<_> = restored.recent.into_iter().collect();
        assert_eq!(vec![3, 4], actual);
        assert_eq!(2, restored.evicted_ids);
        let mut actual: Vec<_> = restored.seen.keys().copied().collect();
        actual.sort_unstable();
        assert_eq!(vec![3, 4], actual);
        Ok(())
    }

    #[tokio::test]
    async fn log_bandwidth_is_shared_across_readers() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            payload_bytes: 1000,
            log_read_bytes_per_second: 10_000,
            ..Profile::default()
        });
        let store = LogStore::new().with_profile(profile.clone(), 0);
        let log = openraft::testing::log_id::<TypeConfig>(1, 0, 1);
        let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(profile.request(1)));
        let bytes = entry_bytes(&entry);
        store.log.write().await.insert(1, entry.clone());
        let started = Instant::now();
        let (first, second) = tokio::join!(
            store.profiled_read(1..2, Duration::ZERO),
            store.profiled_read(1..2, Duration::ZERO)
        );
        assert_eq!(vec![entry.clone()], first?);
        assert_eq!(vec![entry], second?);
        let elapsed = started.elapsed();
        let expected = transfer_time(bytes * 2, profile.log_read_bytes_per_second);
        assert!(elapsed >= expected);
        let report = store.profile_report();
        assert_eq!(2, report["cache_misses"]);
        assert_eq!(bytes * 2, report["logical_read_bytes"]);
        Ok(())
    }

    #[tokio::test]
    async fn newly_appended_tail_entries_hit_the_cache() -> anyhow::Result<()> {
        let profile = Arc::new(Profile {
            profile: true,
            cache_entries: 1,
            log_read_bytes_per_second: 10_000,
            ..Profile::default()
        });
        let store = LogStore::new().with_profile(profile.clone(), 0);
        for index in 1..=2 {
            let log = openraft::testing::log_id::<TypeConfig>(1, 0, index);
            let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(profile.request(index)));
            store.log.write().await.insert(index, entry);
        }
        let old = store.profiled_read(1..2, Duration::ZERO).await?;
        assert_eq!(1, old[0].log_id.index());
        let fresh = store.profiled_read(2..3, Duration::ZERO).await?;
        assert_eq!(2, fresh[0].log_id.index());
        let report = store.profile_report();
        assert_eq!(1, report["cache_hits"]);
        assert_eq!(1, report["cache_misses"]);
        assert_eq!(entry_bytes(&old[0]), report["miss_read_bytes"]);
        assert_eq!(0, report["delay"]["requested_and_elapsed_ns"][1][0]);
        Ok(())
    }
}
