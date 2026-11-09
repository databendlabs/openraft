use std::io;
use std::sync::Arc;

use futures::Stream;
use futures::TryStreamExt;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use tokio::sync::Mutex;

use crate::LogId;
use crate::Snapshot;
use crate::SnapshotMeta;
use crate::StoredMembership;
use crate::TypeConfig;
use crate::archive_buffer::ArchiveBuffer;
use crate::codec::Codec;
use crate::payload::Payload;
use crate::state::State;

/// Demonstration store: memory-only logs/state, not durable across process exits.
#[derive(Clone, Default)]
pub(crate) struct StateMachine {
    state: Arc<Mutex<State>>,
    snapshot: Arc<Mutex<Option<Snapshot>>>,
}

impl StateMachine {
    pub(crate) async fn get(&self, key: &str) -> Option<ArchiveBuffer> {
        self.state.lock().await.values.get(key).cloned()
    }
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachine {
    type SnapshotData = std::io::Cursor<Vec<u8>>;

    async fn build_snapshot(&mut self) -> io::Result<Snapshot> {
        let state = self.state.lock().await;
        let meta = SnapshotMeta {
            last_log_id: state.last_applied,
            last_membership: state.membership.clone(),
        };
        let bytes = Codec::encode(&*state)?;
        let snapshot = Snapshot {
            meta,
            snapshot: std::io::Cursor::new(bytes),
        };
        // Keep the snapshot and its view consistent with each other.
        *self.snapshot.lock().await = Some(snapshot.clone());
        Ok(snapshot)
    }
}

impl RaftStateMachine<TypeConfig> for StateMachine {
    type SnapshotData = std::io::Cursor<Vec<u8>>;
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> io::Result<(Option<LogId>, StoredMembership)> {
        let state = self.state.lock().await;
        Ok((state.last_applied, state.membership.clone()))
    }

    async fn apply<S>(&mut self, mut entries: S) -> io::Result<()>
    where S: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        let mut state = self.state.lock().await;
        while let Some((entry, responder)) = entries.try_next().await? {
            match entry.payload {
                Payload::Blank => {}
                Payload::Normal(buffer) => {
                    // Only the map key is copied. The value is read in-place from ArchivedCommand;
                    // storing the buffer preserves that representation after apply finishes.
                    let key = buffer.archived().key.as_str().to_owned();
                    state.values.insert(key, buffer);
                }
                Payload::Membership(membership) => {
                    state.membership = StoredMembership::new(Some(entry.log_id), membership);
                }
            }
            state.last_applied = Some(entry.log_id);
            if let Some(responder) = responder {
                responder.send(());
            }
        }
        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: Self::SnapshotData) -> io::Result<()> {
        let bytes = snapshot.into_inner();
        let state: State = Codec::decode(&bytes)?;
        if state.last_applied != meta.last_log_id || state.membership != meta.last_membership {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "snapshot metadata mismatch"));
        }
        let mut current = self.state.lock().await;
        *current = state;
        *self.snapshot.lock().await = Some(Snapshot {
            meta: meta.clone(),
            snapshot: std::io::Cursor::new(bytes),
        });
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> io::Result<Option<Snapshot>> {
        Ok(self.snapshot.lock().await.clone())
    }
}
