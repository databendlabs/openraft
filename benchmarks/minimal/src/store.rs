//! A minimized store with least cost for benchmarking Openraft.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use futures::StreamExt;
use futures::future::Either;
use futures::stream;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::alias::DefaultEntryOf;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::entry::RaftEntry;
use openraft::impls::EntryPayload;
use openraft::impls::Vote;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClientRequest {}

impl fmt::Display for ClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClientRequest {{}}")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse {}

pub type NodeId = u64;
pub type SnapshotData = Cursor<Vec<u8>>;

/// Choose a LeaderId implementation by feature flag.
mod leader_id_mode {
    #[cfg(not(feature = "single-term-leader"))]
    pub use openraft::impls::leader_id_adv::LeaderId;
    #[cfg(feature = "single-term-leader")]
    pub use openraft::impls::leader_id_std::LeaderId;
}

type LeaderId = leader_id_mode::LeaderId<u64, u64>;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct TypeConfig;

impl RaftTypeConfig for TypeConfig {
    type D = ClientRequest;
    type R = ClientResponse;
    type NodeId = u64;
    type Node = ();
    type Term = u64;
    type LeaderId = LeaderId;
    type Vote = Vote<Self::LeaderId>;
    type Membership = openraft::impls::Membership<Self::NodeId, Self::Node>;
    type MembershipEdit = openraft::impls::ChangeMembers<Self::NodeId, Self::Node>;
    type Payload = EntryPayload<Self::D, Self::Membership>;
    type Entry = DefaultEntryOf<Self>;
    type Responder<T>
        = openraft::impls::OneshotResponder<Self, T>
    where T: Send + 'static;
    type Batch<T>
        = openraft::impls::InlineBatch<T>
    where T: Send + 'static;
    type AsyncRuntime = openraft::impls::TokioRuntime;
    type ErrorSource = openraft::AnyError;
}

#[derive(Debug)]
pub struct StoredSnapshot {
    pub meta: SnapshotMetaOf<TypeConfig>,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachine {
    pub last_applied_log: Option<LogIdOf<TypeConfig>>,
    pub last_membership: StoredMembershipOf<TypeConfig>,
}

pub struct LogStore {
    vote: RwLock<Option<Vote<LeaderId>>>,
    log: RwLock<BTreeMap<u64, EntryOf<TypeConfig>>>,
    last_purged_log_id: RwLock<Option<LogIdOf<TypeConfig>>>,
    append_delay: Duration,
    flush_delay: Duration,
    /// Delay before each log read issued by the state-machine apply path.
    apply_read_delay: Duration,
}

impl LogStore {
    pub fn new() -> Self {
        let log = RwLock::new(BTreeMap::new());

        Self {
            vote: RwLock::new(None),
            log,
            last_purged_log_id: RwLock::new(None),
            append_delay: Duration::ZERO,
            flush_delay: Duration::ZERO,
            apply_read_delay: Duration::ZERO,
        }
    }

    /// Create a store that delays append submission and completes flushing asynchronously.
    pub fn with_delays(append_delay: Duration, flush_delay: Duration) -> Self {
        let mut store = Self::new();
        store.append_delay = append_delay;
        store.flush_delay = flush_delay;
        store
    }

    /// Delay every `entries_stream()` call, which only the state-machine apply path issues.
    ///
    /// Replication reads go through `leader_bounded_stream()` and are not delayed.
    pub fn with_apply_read_delay(mut self, delay: Duration) -> Self {
        self.apply_read_delay = delay;
        self
    }

    pub async fn new_async() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StateMachineStore {
    sm: RwLock<StateMachine>,
    current_snapshot: RwLock<Option<StoredSnapshot>>,
    /// Simulated I/O time of each `apply()` call; responses are sent after it elapses.
    apply_io_delay: Duration,
}

impl StateMachineStore {
    pub fn new() -> Self {
        Self {
            sm: RwLock::new(StateMachine::default()),
            current_snapshot: RwLock::new(None),
            apply_io_delay: Duration::ZERO,
        }
    }

    /// Delay the end of every `apply()` call, after its entries are consumed, and send the
    /// client responses only after the delay.
    pub fn with_apply_io_delay(mut self, delay: Duration) -> Self {
        self.apply_io_delay = delay;
        self
    }
}

impl Default for StateMachineStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RaftLogReader<TypeConfig> for Arc<LogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<EntryOf<TypeConfig>>, io::Error> {
        let mut entries = vec![];
        {
            let log = self.log.read().await;
            for (_, ent) in log.range(range.clone()) {
                entries.push(ent.clone());
            }
        };

        Ok(entries)
    }

    async fn entries_stream<RB>(
        &mut self,
        range: RB,
    ) -> impl Stream<Item = Result<EntryOf<TypeConfig>, io::Error>> + OptionalSend
    where
        RB: RangeBounds<u64> + Clone + Debug + OptionalSend,
    {
        if let Err(error) = blocking_delay(self.apply_read_delay).await {
            return Either::Right(stream::iter([Err(error)]));
        }
        match self.try_get_log_entries(range).await {
            Ok(entries) => Either::Left(stream::iter(entries).map(Ok)),
            Err(error) => Either::Right(stream::iter([Err(error)])),
        }
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<LeaderId>>, io::Error> {
        Ok(*self.vote.read().await)
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotData = SnapshotData;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<SnapshotOf<TypeConfig, SnapshotData>, io::Error> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_json::to_vec(&*sm).map_err(io::Error::other)?;

            last_applied_log = sm.last_applied_log;
            last_membership = sm.last_membership.clone();
        }

        let snapshot_size = data.len();

        let meta = SnapshotMetaOf::<TypeConfig> {
            last_log_id: last_applied_log,
            last_membership,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        {
            let mut current_snapshot = self.current_snapshot.write().await;
            *current_snapshot = Some(snapshot);
        }

        tracing::info!(snapshot_size, "log compaction complete");

        Ok(SnapshotOf::<TypeConfig, SnapshotData> {
            meta,
            snapshot: Cursor::new(data),
        })
    }
}

impl RaftLogStorage<TypeConfig> for Arc<LogStore> {
    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        let log = self.log.read().await;
        let last = log.iter().next_back().map(|(_, ent)| ent.log_id());

        let last_purged = *self.last_purged_log_id.read().await;

        let last = last.or(last_purged);

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<LeaderId>) -> Result<(), io::Error> {
        let mut v = self.vote.write().await;
        *v = Some(*vote);
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<TypeConfig>>) -> Result<(), io::Error> {
        let start_index = match last_log_id {
            Some(log_id) => log_id.index() + 1,
            None => 0,
        };

        let mut log = self.log.write().await;
        log.split_off(&start_index);

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn purge(&mut self, log_id: LogIdOf<TypeConfig>) -> Result<(), io::Error> {
        {
            let mut p = self.last_purged_log_id.write().await;
            *p = Some(log_id);
        }

        let mut log = self.log.write().await;
        *log = log.split_off(&(log_id.index() + 1));

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip_all)]
    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where I: IntoIterator<Item = EntryOf<TypeConfig>> + Send {
        {
            let mut log = self.log.write().await;
            log.extend(entries.into_iter().map(|entry| (entry.index(), entry)));
        }
        if !self.append_delay.is_zero() {
            tokio::time::sleep(self.append_delay).await;
        }
        if self.flush_delay.is_zero() {
            callback.io_completed(Ok(()));
        } else {
            let deadline = tokio::time::Instant::now() + self.flush_delay;
            tokio::spawn(async move {
                tokio::time::sleep_until(deadline).await;
                callback.io_completed(Ok(()));
            });
        }
        Ok(())
    }

    type LogReader = Self;

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogIdOf<TypeConfig>>, StoredMembershipOf<TypeConfig>), io::Error> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        use futures::TryStreamExt;

        let delayed = !self.apply_io_delay.is_zero();
        let mut delayed_responders = Vec::new();
        let mut sm = self.sm.write().await;

        while let Some((entry, responder)) = entries.try_next().await? {
            sm.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank | EntryPayload::Normal(_) => {}
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembershipOf::<TypeConfig>::new(Some(entry.log_id), mem.clone());
                }
            };

            if let Some(responder) = responder {
                if delayed {
                    delayed_responders.push(responder);
                } else {
                    responder.send(ClientResponse {});
                }
            }
        }

        if delayed {
            blocking_delay(self.apply_io_delay).await?;
            for responder in delayed_responders {
                responder.send(ClientResponse {});
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<TypeConfig>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update the state machine.
        {
            let new_sm: StateMachine = serde_json::from_slice(&new_snapshot.data).map_err(io::Error::other)?;
            let mut sm = self.sm.write().await;
            *sm = new_sm;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.write().await;
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<TypeConfig, Self::SnapshotData>>, io::Error> {
        match &*self.current_snapshot.read().await {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(SnapshotOf::<TypeConfig, Self::SnapshotData> {
                    meta: snapshot.meta.clone(),
                    snapshot: Cursor::new(data),
                }))
            }
            None => Ok(None),
        }
    }

    type SnapshotBuilder = Self;

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

/// Occupy a blocking-pool thread for `delay`, like synchronous storage I/O.
///
/// Unlike `tokio::time::sleep()`, which rounds up to the 1 ms timer tick, this keeps
/// sub-millisecond delays accurate.
async fn blocking_delay(delay: Duration) -> Result<(), io::Error> {
    if delay.is_zero() {
        return Ok(());
    }
    let sleep = tokio::task::spawn_blocking(move || std::thread::sleep(delay));
    sleep.await.map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use openraft::type_config::TypeConfigExt;
    use openraft::vote::RaftLeaderIdExt;

    use super::*;

    #[tokio::test]
    async fn entries_are_readable_before_delayed_flush() -> anyhow::Result<()> {
        let store = LogStore::with_delays(Duration::ZERO, Duration::from_millis(2));
        let mut store = Arc::new(store);
        let leader_id = LeaderId::new_committed(1, 0);
        let log_id = LogIdOf::<TypeConfig>::new(leader_id, 0);
        let entry = EntryOf::<TypeConfig>::new_blank(log_id);
        let expected = vec![entry];
        let (tx, mut rx) = TypeConfig::oneshot();
        let callback = IOFlushed::signal(tx);
        store.append(expected.clone(), callback).await?;
        let pending = rx.try_recv();
        let error = pending.unwrap_err();
        assert_eq!(tokio::sync::oneshot::error::TryRecvError::Empty, error);
        let entries = store.try_get_log_entries(0..1).await?;
        assert_eq!(expected, entries);
        let timeout = Duration::from_secs(5);
        let flushed = tokio::time::timeout(timeout, rx).await?;
        flushed??;
        Ok(())
    }

    #[tokio::test]
    async fn apply_read_delay_delays_only_entries_stream() -> anyhow::Result<()> {
        use futures::TryStreamExt;

        let delay = Duration::from_millis(20);
        let store = LogStore::new().with_apply_read_delay(delay);
        let mut store = Arc::new(store);
        let leader_id = LeaderId::new_committed(1, 0);
        let log_id = LogIdOf::<TypeConfig>::new(leader_id, 0);
        let entry = EntryOf::<TypeConfig>::new_blank(log_id);
        let expected = vec![entry];
        let (tx, rx) = TypeConfig::oneshot();
        let callback = IOFlushed::signal(tx);
        store.append(expected.clone(), callback).await?;
        rx.await??;

        let started = tokio::time::Instant::now();
        let entries = store.try_get_log_entries(0..1).await?;
        let undelayed = started.elapsed();
        assert_eq!(expected, entries);
        assert!(undelayed < delay, "try_get_log_entries() took {undelayed:?}");

        let started = tokio::time::Instant::now();
        let stream = store.entries_stream(0..1).await;
        let entries: Vec<_> = stream.try_collect().await?;
        let delayed = started.elapsed();
        assert_eq!(expected, entries);
        assert!(delayed >= delay, "entries_stream() took {delayed:?}");
        Ok(())
    }
}
