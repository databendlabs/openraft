use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;

use bincode;
use openraft::entry::RaftEntryExt;
use openraft::storage::RaftStateMachine;
use openraft::EntryPayload;
use openraft::RaftSnapshotBuilder;
use serde::Deserialize;
use serde::Serialize;

use crate::pb;
use crate::protobuf::Response;
use crate::typ::*;
use crate::TypeConfig;

pub type LogStore = memstore::LogStore<TypeConfig>;

#[derive(Debug)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: Box<SnapshotData>,
}

/// Data contained in the Raft state machine.
///
/// Note that we are using `serde` to serialize the
/// `data`, which has a implementation to be serialized. Note that for this test we set both the key
/// and value as String, but you could set any type of value that has the serialization impl.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachineData {
    pub last_applied: Option<LogId>,

    pub last_membership: StoredMembership,

    /// Application data.
    pub data: BTreeMap<String, String>,
}

impl StateMachineData {
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("Failed to serialize StateMachineData")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

/// Defines a state machine for the Raft cluster. This state machine represents a copy of the
/// data for this node. Additionally, it is responsible for storing the last snapshot of the data.
#[derive(Debug, Default)]
pub struct StateMachineStore {
    /// The Raft state machine.
    pub state_machine: Mutex<StateMachineData>,

    snapshot_idx: Mutex<u64>,

    /// The last received snapshot.
    current_snapshot: Mutex<Option<StoredSnapshot>>,
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot, StorageError> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = self.state_machine.lock().unwrap().clone();

            last_applied_log = state_machine.last_applied;
            last_membership = state_machine.last_membership.clone();
            data = state_machine;
        }

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: Box::new(data.clone()),
        };

        {
            let mut current_snapshot = self.current_snapshot.lock().unwrap();
            *current_snapshot = Some(snapshot);
        }

        Ok(Snapshot {
            meta,
            snapshot: Box::new(data),
        })
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), StorageError> {
        let state_machine = self.state_machine.lock().unwrap();
        Ok((state_machine.last_applied, state_machine.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Response>, StorageError>
    where I: IntoIterator<Item = Entry> {
        let mut res = Vec::new(); //No `with_capacity`; do not know `len` of iterator

        let mut sm = self.state_machine.lock().unwrap();

        for entry in entries {
            let log_id = entry.to_log_id();

            tracing::debug!("replicate to sm: {}", log_id);

            sm.last_applied = Some(log_id);

            match entry.payload {
                None => res.push(Response { value: None }),
                Some(pb::entry::Payload::Normal(req)) => {
                    sm.data.insert(req.key, req.value.clone());
                    res.push(Response { value: Some(req.value) });
                }
                Some(pb::entry::Payload::Membership(mem)) => {
                    sm.last_membership = StoredMembership::new(Some(log_id), mem.into());
                    res.push(Response { value: None })
                }
            };
        }
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<SnapshotData>, StorageError> {
        Ok(Box::default())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: Box<SnapshotData>) -> Result<(), StorageError> {
        tracing::info!("install snapshot");

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot,
        };

        // Update the state machine.
        {
            let updated_state_machine: StateMachineData = *new_snapshot.data.clone();
            let mut state_machine = self.state_machine.lock().unwrap();
            *state_machine = updated_state_machine;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.lock().unwrap();
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, StorageError> {
        match &*self.current_snapshot.lock().unwrap() {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: data,
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}
