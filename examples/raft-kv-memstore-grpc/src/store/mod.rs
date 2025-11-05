use std::fmt::Debug;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use futures::TryStreamExt;
use openraft::entry::RaftEntry;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;

use crate::protobuf as pb;
use crate::protobuf::Response;
use crate::typ::*;
use crate::TypeConfig;

pub type LogStore = mem_log::LogStore<TypeConfig>;

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
    pub state_machine: tokio::sync::Mutex<pb::StateMachineData>,

    snapshot_idx: Mutex<u64>,

    /// The last received snapshot.
    current_snapshot: Mutex<Option<StoredSnapshot>>,
}

impl StateMachineStore {
    /// Generates a unique snapshot ID based on the last applied log and snapshot index.
    fn generate_snapshot_id(last_applied: &Option<LogId>, snapshot_idx: u64) -> String {
        if let Some(last) = last_applied {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        }
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let data;
        let last_applied: Option<LogId>;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = self.state_machine.lock().await.clone();

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

        let snapshot_id = StateMachineStore::generate_snapshot_id(&last_applied, snapshot_idx);

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

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        let sm = self.state_machine.lock().await;

        let last_applied = sm.last_applied.map(|x| x.into());

        let mem = StoredMembership::new(
            sm.last_membership_log_id.map(|x| x.into()),
            sm.last_membership.clone().unwrap_or_default().into(),
        );

        Ok((last_applied, mem))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        let mut sm = self.state_machine.lock().await;

        while let Some((entry, responder)) = entries.try_next().await? {
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

            if let Some(responder) = responder {
                responder.send(Response { value });
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotData, io::Error> {
        Ok(Default::default())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        tracing::info!("install snapshot");

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot,
        };

        // Update the state machine.
        {
            let d: pb::StateMachineData = prost::Message::decode(new_snapshot.data.as_ref())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            let mut state_machine = self.state_machine.lock().await;
            *state_machine = d;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.lock().unwrap();
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
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
