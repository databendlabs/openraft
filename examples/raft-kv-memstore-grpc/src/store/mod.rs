use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::entry::RaftEntry;
use openraft::storage::RaftStateMachine;
use openraft::RaftSnapshotBuilder;

use crate::protobuf as pb;
use crate::protobuf::Response;
use crate::typ::*;
use crate::TypeConfig;

pub type LogStore = memstore::LogStore<TypeConfig>;

#[derive(Debug)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: SnapshotData,
}

/// Defines a state machine for the Raft cluster. This state machine represents a copy of the
/// data for this node. Additionally, it is responsible for storing the last snapshot of the data.
#[derive(Debug, Default)]
pub struct StateMachineStore {
    /// The Raft state machine.
    pub state_machine: Mutex<pb::StateMachineData>,

    snapshot_idx: Mutex<u64>,

    /// The last received snapshot.
    current_snapshot: Mutex<Option<StoredSnapshot>>,
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot, StorageError> {
        let data;
        let last_applied: Option<LogId>;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = self.state_machine.lock().unwrap().clone();

            last_applied = state_machine.last_applied.map(From::from);
            let last_membership_log_id = state_machine.last_membership_log_id.map(|log_id| log_id.into());
            let membership = state_machine.last_membership.clone().unwrap_or_default().into();
            last_membership = StoredMembership::new(last_membership_log_id, membership);

            data = prost::Message::encode_to_vec(&state_machine);
        }

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let snapshot_id = if let Some(last) = &last_applied {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership,
            snapshot_id,
        };

        let stored = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        // Emulation of storing snapshot locally
        {
            let mut current_snapshot = self.current_snapshot.lock().unwrap();
            *current_snapshot = Some(stored);
        }

        Ok(Snapshot { meta, snapshot: data })
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), StorageError> {
        let sm = self.state_machine.lock().unwrap();

        let last_applied = sm.last_applied.map(|x| x.into());

        let mem = StoredMembership::new(
            sm.last_membership_log_id.map(|x| x.into()),
            sm.last_membership.clone().unwrap_or_default().into(),
        );

        Ok((last_applied, mem))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Response>, StorageError>
    where I: IntoIterator<Item = Entry> {
        let mut res = Vec::new(); //No `with_capacity`; do not know `len` of iterator

        let mut sm = self.state_machine.lock().unwrap();

        for entry in entries {
            let log_id = entry.log_id();

            tracing::debug!("replicate to sm: {}", log_id);

            sm.last_applied = Some(log_id.into());

            let value = if let Some(req) = entry.app_data {
                sm.data.insert(req.key, req.value.clone());
                Some(req.value)
            } else if let Some(mem) = entry.membership {
                sm.last_membership_log_id = Some(log_id.into());
                sm.last_membership = Some(mem);
                None
            } else {
                None
            };

            res.push(Response { value });
        }
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotData, StorageError> {
        Ok(Default::default())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), StorageError> {
        tracing::info!("install snapshot");

        // Update the state machine.
        {
            let d: pb::StateMachineData =
                prost::Message::decode(snapshot.as_ref()).map_err(|e| StorageError::read_snapshot(None, &e))?;

            let mut state_machine = self.state_machine.lock().unwrap();
            *state_machine = d;
        }

        let snapshot = Snapshot {
            meta: meta.clone(),
            snapshot: Cursor::new(snapshot),
        };

        self.save_snapshot(&snapshot).await?;

        Ok(())
    }

    async fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError> {
        let new_snapshot = StoredSnapshot {
            meta: snapshot.meta.clone(),
            data: snapshot.snapshot.clone().into_inner(),
        };

        let mut current = self.current_snapshot.lock().unwrap();

        // Only save it if the new snapshot contains more recent data than the current snapshot.

        let current_last = current.as_ref().and_then(|s| s.meta.last_log_id);
        if new_snapshot.meta.last_log_id <= current_last {
            return Ok(());
        }

        *current = Some(new_snapshot);

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
