#![deny(unused_crate_dependencies)]
#[cfg(test)]
mod test;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::OptionalSend;
use openraft::SnapshotMeta;
use openraft::StoredMembership;
use openraft::Vote;
use openraft::alias::SnapshotDataOf;
use openraft::entry::RaftEntry;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

/// A `NodeId` newtype whose `Display` produces `"Node[N]"` instead of plain `"N"`.
///
/// This is the key difference from the default `u64`-based `NodeId`: it lets the
/// storage test suite be exercised with a non-trivial display format so that any
/// place that accidentally relies on `NodeId` printing as a bare integer is caught.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize
)]
pub struct NodeId(pub u64);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node[{}]", self.0)
    }
}

impl From<u64> for NodeId {
    fn from(v: u64) -> Self {
        NodeId(v)
    }
}

/// Minimal application request type. The storage test suite does not exercise application data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppReq;

impl std::fmt::Display for AppReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppReq")
    }
}

openraft::declare_raft_types!(
    pub TypeConfig:
        D = AppReq,
        R = (),
        NodeId = NodeId,
        Node = (),
        LeaderId = openraft::impls::leader_id_adv::LeaderId<TypeConfig>,
);

// ── State machine ────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
struct StateMachine {
    last_applied_log: Option<LogId<TypeConfig>>,
    last_membership: StoredMembership<TypeConfig>,
}

pub struct MemStateMachine {
    sm: RwLock<StateMachine>,
    snapshot_idx: Mutex<u64>,
    current_snapshot: RwLock<Option<(SnapshotMeta<TypeConfig>, Vec<u8>)>>,
}

impl MemStateMachine {
    fn new() -> Self {
        Self {
            sm: RwLock::new(StateMachine::default()),
            snapshot_idx: Mutex::new(0),
            current_snapshot: RwLock::new(None),
        }
    }
}

// ── Log store ────────────────────────────────────────────────────────────────

pub struct MemLogStore {
    last_purged_log_id: RwLock<Option<LogId<TypeConfig>>>,
    committed: RwLock<Option<LogId<TypeConfig>>>,
    log: RwLock<BTreeMap<u64, String>>,
    vote: RwLock<Option<Vote<TypeConfig>>>,
}

impl MemLogStore {
    fn new() -> Self {
        Self {
            last_purged_log_id: RwLock::new(None),
            committed: RwLock::new(None),
            log: RwLock::new(BTreeMap::new()),
            vote: RwLock::new(None),
        }
    }
}

pub fn new_mem_store() -> (Arc<MemLogStore>, Arc<MemStateMachine>) {
    (Arc::new(MemLogStore::new()), Arc::new(MemStateMachine::new()))
}

// ── RaftLogReader ────────────────────────────────────────────────────────────

impl RaftLogReader<TypeConfig> for Arc<MemLogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, io::Error> {
        let log = self.log.read().await;
        log.range(range)
            .map(|(_, s)| serde_json::from_str(s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)))
            .collect()
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<TypeConfig>>, io::Error> {
        Ok(*self.vote.read().await)
    }
}

// ── RaftLogStorage ───────────────────────────────────────────────────────────

impl RaftLogStorage<TypeConfig> for Arc<MemLogStore> {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        let log = self.log.read().await;
        let last = match log.iter().next_back() {
            None => *self.last_purged_log_id.read().await,
            Some((_, s)) => {
                let ent: Entry<TypeConfig> =
                    serde_json::from_str(s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Some(ent.log_id())
            }
        };
        Ok(LogState {
            last_purged_log_id: *self.last_purged_log_id.read().await,
            last_log_id: last,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote<TypeConfig>) -> Result<(), io::Error> {
        *self.vote.write().await = Some(*vote);
        Ok(())
    }

    async fn save_committed(&mut self, committed: Option<LogId<TypeConfig>>) -> Result<(), io::Error> {
        *self.committed.write().await = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<TypeConfig>>, io::Error> {
        Ok(*self.committed.read().await)
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend {
        let mut log = self.log.write().await;
        for entry in entries {
            let s = serde_json::to_string(&entry).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            log.insert(entry.index(), s);
        }
        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogId<TypeConfig>>) -> Result<(), io::Error> {
        let start = last_log_id.map_or(0, |id| id.index() + 1);
        let mut log = self.log.write().await;
        let keys: Vec<_> = log.range(start..).map(|(k, _)| *k).collect();
        for k in keys {
            log.remove(&k);
        }
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<TypeConfig>) -> Result<(), io::Error> {
        *self.last_purged_log_id.write().await = Some(log_id);
        let mut log = self.log.write().await;
        let keys: Vec<_> = log.range(..=log_id.index()).map(|(k, _)| *k).collect();
        for k in keys {
            log.remove(&k);
        }
        Ok(())
    }
}

// ── RaftSnapshotBuilder ──────────────────────────────────────────────────────

impl RaftSnapshotBuilder<TypeConfig> for Arc<MemStateMachine> {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, io::Error> {
        let sm = self.sm.read().await;
        let data = serde_json::to_vec(&*sm).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let snapshot_idx = {
            let mut idx = self.snapshot_idx.lock().unwrap();
            *idx += 1;
            *idx
        };

        let snapshot_id = match sm.last_applied_log {
            Some(last) => format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx),
            None => format!("--{}", snapshot_idx),
        };

        let meta = SnapshotMeta {
            last_log_id: sm.last_applied_log,
            last_membership: sm.last_membership.clone(),
            snapshot_id,
        };

        *self.current_snapshot.write().await = Some((meta.clone(), data.clone()));

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(data),
        })
    }
}

// ── RaftStateMachine ─────────────────────────────────────────────────────────

impl RaftStateMachine<TypeConfig> for Arc<MemStateMachine> {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId<TypeConfig>>, StoredMembership<TypeConfig>), io::Error> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        use futures::TryStreamExt;

        let mut sm = self.sm.write().await;
        while let Some((entry, responder)) = entries.try_next().await? {
            sm.last_applied_log = Some(entry.log_id);
            if let EntryPayload::Membership(ref mem) = entry.payload {
                sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
            }
            if let Some(r) = responder {
                r.send(());
            }
        }
        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotDataOf<TypeConfig>, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<TypeConfig>,
        snapshot: SnapshotDataOf<TypeConfig>,
    ) -> Result<(), io::Error> {
        let new_sm: StateMachine =
            serde_json::from_slice(snapshot.get_ref()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        *self.sm.write().await = new_sm;
        *self.current_snapshot.write().await = Some((meta.clone(), snapshot.into_inner()));
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, io::Error> {
        Ok(
            self.current_snapshot.read().await.as_ref().map(|(meta, data)| Snapshot {
                meta: meta.clone(),
                snapshot: Cursor::new(data.clone()),
            }),
        )
    }
}
