//! In-memory storage for turmoil tests, with the faults of a real disk.
//!
//! The log store keeps two copies of everything it holds: `live`, what a
//! reader sees, and `durable`, what survives a power loss. Under
//! [`StorageModel::durable`] both move together, as in the harness before
//! these faults existed. With `async_flush`, each write lands in `live` at
//! once and joins a queue; a flush task moves the queue to `durable` in
//! submission order and only then reports the appends flushed. See
//! [`StorageModel`] for what the openraft storage contract allows.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures::Stream;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::alias::LogIdListOf;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use rand::RngExt;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde::Deserialize;
use serde::Serialize;

use crate::storage_model::StorageModel;
use crate::typ::*;

/// A versioned value in the KV store.
///
/// `log_id` is the id of the log entry that last wrote the key. It exposes the
/// key's position in the total log order, which is what the client oracle
/// compares to detect stale or non-monotonic reads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValueMeta {
    pub value: String,
    /// Serial of the client write that produced this value.
    pub serial: u64,
    /// Log id of the entry that last wrote this key.
    pub log_id: LogId,
}

/// State machine data.
///
/// `data` is a `BTreeMap` (not `HashMap`) so snapshot bytes serialize in a
/// deterministic order.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateMachineData {
    pub last_applied: Option<LogId>,
    pub last_membership: StoredMembership,
    /// Key-value store.
    pub data: BTreeMap<String, ValueMeta>,
}

/// An entry a log store accepted, recorded for the log-id uniqueness check.
#[derive(Debug, Clone)]
pub struct AppendedEntry {
    pub node: NodeId,
    pub log_id: LogId,
    /// The serialized payload: two entries are the same if these bytes are.
    pub payload: Vec<u8>,
}

/// Serialize an entry's payload for comparison across nodes.
pub fn payload_bytes(entry: &Entry) -> Vec<u8> {
    serde_json::to_vec(&entry.payload).expect("entry payload serializes")
}

/// Everything a log store holds, as one copy: what a reader sees, or what is
/// on disk.
#[derive(Debug, Clone, Default)]
struct LogDisk {
    vote: Option<Vote>,
    committed: Option<LogId>,
    last_purged: Option<LogId>,
    log: BTreeMap<u64, Entry>,
}

/// One write to a log store, kept until it is flushed.
enum LogWrite {
    Append(Vec<Entry>),
    /// Keep entries up to and including this log id.
    Truncate(Option<LogId>),
    Purge(LogId),
    Vote(Vote),
    Committed(Option<LogId>),
}

/// Who waits for a write to be flushed.
enum FlushWaiter {
    None,
    Append(IOFlushed<TypeConfig>),
    Vote(tokio::sync::oneshot::Sender<()>),
}

impl LogDisk {
    fn apply(&mut self, write: &LogWrite) {
        match write {
            LogWrite::Append(entries) => {
                for entry in entries {
                    self.log.insert(entry.log_id.index, entry.clone());
                }
            }
            LogWrite::Truncate(last_log_id) => {
                let start_index = match last_log_id {
                    Some(log_id) => log_id.index + 1,
                    None => 0,
                };
                self.log.split_off(&start_index);
            }
            LogWrite::Purge(log_id) => {
                self.last_purged = Some(*log_id);
                self.log = self.log.split_off(&(log_id.index + 1));
            }
            LogWrite::Vote(vote) => self.vote = Some(*vote),
            LogWrite::Committed(committed) => self.committed = *committed,
        }
    }

    fn log_id_list(&self) -> LogIdListOf<TypeConfig> {
        LogIdListOf::<TypeConfig>::new(self.last_purged, self.log.values().map(|entry| entry.log_id))
    }
}

struct LogStoreInner {
    /// What a reader sees: every write submitted so far.
    live: LogDisk,
    /// What survives a power loss: every write flushed so far.
    durable: LogDisk,
    /// Writes submitted but not flushed, oldest first.
    unflushed: VecDeque<(LogWrite, FlushWaiter)>,
    /// Bumped by every crash, so a flush task left from before it stops.
    epoch: u64,
    /// Whether a flush task is running for the current epoch.
    flushing: bool,
    /// Entries appended since the harness last collected them.
    appended: Vec<AppendedEntry>,
    rng: StdRng,
}

/// In-memory log store.
pub struct LogStore {
    node_id: NodeId,
    model: StorageModel,
    inner: Mutex<LogStoreInner>,
}

impl LogStore {
    pub fn new(node_id: NodeId, model: StorageModel, seed: u64) -> Self {
        Self {
            node_id,
            model,
            inner: Mutex::new(LogStoreInner {
                live: LogDisk::default(),
                durable: LogDisk::default(),
                unflushed: VecDeque::new(),
                epoch: 0,
                flushing: false,
                appended: Vec::new(),
                rng: StdRng::seed_from_u64(seed),
            }),
        }
    }

    /// The log ids on disk: what this node would restart with after a power
    /// loss.
    pub fn log_id_list(&self) -> LogIdListOf<TypeConfig> {
        self.inner.lock().unwrap().durable.log_id_list()
    }

    /// Take the entries appended since the last call.
    pub fn take_appended(&self) -> Vec<AppendedEntry> {
        std::mem::take(&mut self.inner.lock().unwrap().appended)
    }

    /// The payload of the entry a reader sees at `log_id`, if this store
    /// holds exactly that log id.
    pub fn payload_at(&self, log_id: &LogId) -> Option<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        inner.live.log.get(&log_id.index).filter(|e| e.log_id == *log_id).map(payload_bytes)
    }

    /// Every entry a reader sees, as `(log id, payload)`.
    pub fn entries(&self) -> Vec<(LogId, Vec<u8>)> {
        let inner = self.inner.lock().unwrap();
        inner.live.log.values().map(|e| (e.log_id, payload_bytes(e))).collect()
    }

    /// The process holding this store crashed. Its flush task is gone and
    /// nobody waits for the unflushed writes any more. A power loss drops
    /// them; otherwise the page cache still reaches the disk.
    pub fn crash(&self, power_loss: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.epoch += 1;
        inner.flushing = false;
        let unflushed = std::mem::take(&mut inner.unflushed);
        if power_loss {
            inner.live = inner.durable.clone();
        } else {
            for (write, _) in &unflushed {
                inner.durable.apply(write);
            }
        }
    }

    /// Submit one write: readable at once, durable now or once flushed.
    ///
    /// Without `async_flush` the write is durable before this returns, and
    /// `waiter` hears so here.
    fn submit(self: &Arc<Self>, write: LogWrite, waiter: FlushWaiter) {
        let mut inner = self.inner.lock().unwrap();
        inner.live.apply(&write);
        if let LogWrite::Append(entries) = &write {
            let node = self.node_id;
            let appended: Vec<_> = entries
                .iter()
                .map(|e| AppendedEntry {
                    node,
                    log_id: e.log_id,
                    payload: payload_bytes(e),
                })
                .collect();
            inner.appended.extend(appended);
        }

        if !self.model.async_flush {
            inner.durable.apply(&write);
            drop(inner);
            match waiter {
                FlushWaiter::None => {}
                FlushWaiter::Append(callback) => callback.io_completed(Ok(())),
                FlushWaiter::Vote(tx) => {
                    let _ = tx.send(());
                }
            }
            return;
        }

        inner.unflushed.push_back((write, waiter));
        if !inner.flushing {
            inner.flushing = true;
            let epoch = inner.epoch;
            tokio::spawn(self.clone().flush_loop(epoch));
        }
    }

    /// Flush in rounds, like `fsync`: a round covers the writes submitted
    /// when it started and completes one flush latency later.
    async fn flush_loop(self: Arc<Self>, epoch: u64) {
        loop {
            let (round, delay) = {
                let mut inner = self.inner.lock().unwrap();
                let (lo, hi) = self.model.flush_delay_ms;
                let stall = self.model.flush_stall_chance;
                let delay = if stall > 0.0 && inner.rng.random_bool(stall) {
                    inner.rng.random_range(hi..=self.model.flush_stall_ms.max(hi))
                } else {
                    inner.rng.random_range(lo..=hi)
                };
                (inner.unflushed.len(), delay)
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;

            let mut flushed = Vec::with_capacity(round);
            let jitter: Vec<u64>;
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.epoch != epoch {
                    return;
                }
                for _ in 0..round {
                    let (write, waiter) = inner.unflushed.pop_front().expect("only a crash shrinks the queue");
                    inner.durable.apply(&write);
                    flushed.push(waiter);
                }
                let max = self.model.callback_jitter_ms;
                jitter = flushed.iter().map(|_| if max == 0 { 0 } else { inner.rng.random_range(0..=max) }).collect();
                if inner.unflushed.is_empty() {
                    inner.flushing = false;
                }
            }

            for (waiter, delay) in flushed.into_iter().zip(jitter) {
                match waiter {
                    FlushWaiter::None => {}
                    FlushWaiter::Vote(tx) => {
                        let _ = tx.send(());
                    }
                    FlushWaiter::Append(callback) if delay == 0 => callback.io_completed(Ok(())),
                    FlushWaiter::Append(callback) => {
                        // Durable already; only the report is late, and it
                        // may overtake the report of a later append.
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                            callback.io_completed(Ok(()));
                        });
                    }
                }
            }

            if !self.inner.lock().unwrap().flushing {
                return;
            }
        }
    }
}

impl RaftLogReader<TypeConfig> for Arc<LogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry>, io::Error> {
        let inner = self.inner.lock().unwrap();
        let entries: Vec<Entry> = inner.live.log.range(range).map(|(_, e)| e.clone()).collect();
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote>, io::Error> {
        Ok(self.inner.lock().unwrap().live.vote)
    }

    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<Entry>, io::Error> {
        self.try_get_log_entries(start..end).await
    }
}

impl RaftLogStorage<TypeConfig> for Arc<LogStore> {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        let inner = self.inner.lock().unwrap();
        let last_purged = inner.live.last_purged;
        let last = inner.live.log.iter().next_back().map(|(_, e)| e.log_id);

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last.or(last_purged),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    /// Durable before it returns, and so is every write before it.
    async fn save_vote(&mut self, vote: &Vote) -> Result<(), io::Error> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.submit(LogWrite::Vote(*vote), FlushWaiter::Vote(tx));
        if self.model.async_flush {
            rx.await.map_err(|_| io::Error::other("the log store crashed before flushing the vote"))?;
        }
        Ok(())
    }

    /// Needs no flush, as in the example stores: losing it only re-applies
    /// some entries at startup.
    async fn save_committed(&mut self, committed: Option<LogId>) -> Result<(), io::Error> {
        self.submit(LogWrite::Committed(committed), FlushWaiter::None);
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId>, io::Error> {
        Ok(self.inner.lock().unwrap().live.committed)
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where I: IntoIterator<Item = Entry> + OptionalSend {
        self.submit(
            LogWrite::Append(entries.into_iter().collect()),
            FlushWaiter::Append(callback),
        );
        Ok(())
    }

    /// Needs no flush: after a crash openraft truncates again.
    async fn truncate_after(&mut self, last_log_id: Option<LogId>) -> Result<(), io::Error> {
        self.submit(LogWrite::Truncate(last_log_id), FlushWaiter::None);
        Ok(())
    }

    /// Needs no flush: after a crash openraft purges again.
    async fn purge(&mut self, log_id: LogId) -> Result<(), io::Error> {
        self.submit(LogWrite::Purge(log_id), FlushWaiter::None);
        Ok(())
    }
}

/// In-memory state machine, durable when `apply` returns.
pub struct StateMachine {
    data: Mutex<StateMachineData>,
    snapshot: Mutex<Option<(SnapshotMeta, Vec<u8>)>>,
    /// Upper bound of the delay before each apply batch, in ms.
    apply_delay_ms: u64,
    rng: Mutex<StdRng>,
}

impl StateMachine {
    pub fn new(model: &StorageModel, seed: u64) -> Self {
        Self {
            data: Mutex::new(StateMachineData::default()),
            snapshot: Mutex::new(None),
            apply_delay_ms: model.apply_delay_ms,
            rng: Mutex::new(StdRng::seed_from_u64(seed)),
        }
    }

    /// Get a clone of the current state machine data.
    pub fn get_data(&self) -> StateMachineData {
        self.data.lock().unwrap().clone()
    }

    /// Read a single key without cloning the whole map.
    pub fn get_key(&self, key: &str) -> Option<ValueMeta> {
        self.data.lock().unwrap().data.get(key).cloned()
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachine> {
    type SnapshotData = SnapshotData;

    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let data = self.data.lock().unwrap();
        let snapshot_data = serde_json::to_vec(&*data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let meta = SnapshotMeta {
            last_log_id: data.last_applied,
            last_membership: data.last_membership.clone(),
        };

        *self.snapshot.lock().unwrap() = Some((meta.clone(), snapshot_data.clone()));

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(snapshot_data),
        })
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachine> {
    type SnapshotData = crate::typ::SnapshotData;

    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        let data = self.data.lock().unwrap();
        Ok((data.last_applied, data.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        use futures::TryStreamExt;

        // Collect all entries first to avoid holding lock across await
        let mut collected = vec![];
        while let Some(entry_responder) = entries.try_next().await? {
            collected.push(entry_responder);
        }

        tracing::info!("Applying {} entries to state machine", collected.len());

        if self.apply_delay_ms > 0 {
            let delay = self.rng.lock().unwrap().random_range(0..=self.apply_delay_ms);
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        // Now apply all entries while holding the lock
        let mut data = self.data.lock().unwrap();

        for (entry, responder) in collected {
            data.last_applied = Some(entry.log_id);

            let response = match entry.payload {
                EntryPayload::Blank => Response { prev: None },
                EntryPayload::Normal(ref req) => {
                    let prev = data.data.insert(req.key.clone(), ValueMeta {
                        value: req.value.clone(),
                        serial: req.serial,
                        log_id: entry.log_id,
                    });
                    Response { prev }
                }
                EntryPayload::Membership(ref mem) => {
                    data.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    Response { prev: None }
                }
            };

            if let Some(responder) = responder {
                responder.send(response);
            }
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        let new_data: StateMachineData =
            serde_json::from_slice(snapshot.get_ref()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        *self.data.lock().unwrap() = new_data;
        *self.snapshot.lock().unwrap() = Some((meta.clone(), snapshot.into_inner()));

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
        let snap = self.snapshot.lock().unwrap();
        match &*snap {
            Some((meta, data)) => Ok(Some(Snapshot {
                meta: meta.clone(),
                snapshot: Cursor::new(data.clone()),
            })),
            None => Ok(None),
        }
    }
}

/// Create a node's log store and state machine.
///
/// The log store draws its flush latencies from `seed`, the state machine its
/// apply delays from `seed + 1`.
pub fn new_store(node_id: NodeId, model: &StorageModel, seed: u64) -> (Arc<LogStore>, Arc<StateMachine>) {
    (
        Arc::new(LogStore::new(node_id, model.clone(), seed)),
        Arc::new(StateMachine::new(model, seed.wrapping_add(1))),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use openraft::EntryPayload;
    use openraft::storage::IOFlushed;
    use openraft::storage::RaftLogReader;
    use openraft::storage::RaftLogStorage;
    use openraft::type_config::TypeConfigExt;
    use openraft::vote::RaftLeaderId;

    use super::LogStore;
    use crate::storage_model::StorageModel;
    use crate::typ::*;

    fn async_flush(flush_delay_ms: u64) -> StorageModel {
        StorageModel {
            async_flush: true,
            flush_delay_ms: (flush_delay_ms, flush_delay_ms),
            flush_stall_chance: 0.0,
            flush_stall_ms: 0,
            callback_jitter_ms: 0,
            power_loss_chance: 1.0,
            apply_delay_ms: 0,
        }
    }

    fn log_id(index: u64) -> LogId {
        LogId::new(openraft::impls::leader_id_adv::LeaderId::new(1, 1), index)
    }

    fn blank(index: u64) -> Entry {
        Entry {
            log_id: log_id(index),
            payload: EntryPayload::Blank,
        }
    }

    async fn last_index(store: &mut Arc<LogStore>) -> Option<u64> {
        store.get_log_state().await.unwrap().last_log_id.map(|id| id.index)
    }

    #[tokio::test]
    async fn power_loss_drops_unflushed_appends() {
        let mut store = Arc::new(LogStore::new(1, async_flush(1_000), 0));
        let (tx, mut rx) = TypeConfig::oneshot();
        store.append([blank(1), blank(2)], IOFlushed::signal(tx)).await.unwrap();

        // Readable at once, not on disk, not reported flushed.
        assert_eq!(last_index(&mut store).await, Some(2));
        assert_eq!(store.log_id_list().get(2), None);
        assert!(rx.try_recv().is_err());

        store.crash(true);
        assert_eq!(last_index(&mut store).await, None);
    }

    #[tokio::test]
    async fn process_crash_keeps_unflushed_appends() {
        let mut store = Arc::new(LogStore::new(1, async_flush(1_000), 0));
        store.append([blank(1), blank(2)], IOFlushed::noop()).await.unwrap();

        store.crash(false);
        assert_eq!(last_index(&mut store).await, Some(2));
        assert_eq!(store.log_id_list().get(2), Some(log_id(2)));
    }

    #[tokio::test]
    async fn saved_vote_is_durable_with_every_write_before_it() {
        let mut store = Arc::new(LogStore::new(1, async_flush(2), 0));
        store.append([blank(1), blank(2), blank(3)], IOFlushed::noop()).await.unwrap();
        store.truncate_after(Some(log_id(1))).await.unwrap();
        let vote = Vote::new_committed(2, 1);
        store.save_vote(&vote).await.unwrap();

        store.crash(true);
        assert_eq!(store.read_vote().await.unwrap(), Some(vote));
        assert_eq!(last_index(&mut store).await, Some(1));
    }

    #[tokio::test]
    async fn lazy_writes_are_lost_with_unflushed_appends() {
        let mut store = Arc::new(LogStore::new(1, async_flush(2), 0));
        let (tx, rx) = TypeConfig::oneshot();
        store.append([blank(1), blank(2), blank(3)], IOFlushed::signal(tx)).await.unwrap();
        rx.await.unwrap().unwrap();

        // Neither needs a flush; a power loss before the next one undoes both.
        store.truncate_after(Some(log_id(1))).await.unwrap();
        store.save_committed(Some(log_id(1))).await.unwrap();
        store.crash(true);
        assert_eq!(last_index(&mut store).await, Some(3));
        assert_eq!(store.read_committed().await.unwrap(), None);

        // Flushed, they survive.
        store.truncate_after(Some(log_id(1))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        store.crash(true);
        assert_eq!(last_index(&mut store).await, Some(1));
    }

    #[tokio::test]
    async fn durable_model_writes_through() {
        let mut store = Arc::new(LogStore::new(1, StorageModel::durable(), 0));
        let (tx, mut rx) = TypeConfig::oneshot();
        store.append([blank(1)], IOFlushed::signal(tx)).await.unwrap();
        assert!(rx.try_recv().is_ok(), "reported flushed before append returns");

        store.crash(true);
        assert_eq!(last_index(&mut store).await, Some(1));
    }
}
