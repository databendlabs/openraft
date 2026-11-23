//! Provide `StateMachineStore`, an in-memory KV state machine implementation.

mod client_session;

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io;
use std::io::Cursor;
use std::sync::Arc;

pub use client_session::ClientSession;
use futures::Stream;
use futures::TryStreamExt;
use futures::lock::Mutex;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::alias::DefaultEntryOf;
use openraft::alias::EntryPayloadOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::macros::since;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft_legacy::network_v1::SnapshotReceiverFactory;
use serde::Deserialize;
use serde::Serialize;
use types_kv::KvCommand;

#[derive(Debug)]
pub struct StoredSnapshot<C: RaftTypeConfig> {
    pub meta: SnapshotMetaOf<C>,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

/// Data contained in the Raft state machine.
#[since(
    version = "0.10.2",
    change = "add client sessions, which answer a resent write with its first response"
)]
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachineData {
    /// Application data.
    pub data: BTreeMap<String, types_kv::VersionedValue>,

    /// The latest applied write of each client that tags its writes with a
    /// [`types_kv::RequestId`], by client ID.
    ///
    /// It belongs to the replicated state: every node applies the same writes to it, and a
    /// snapshot carries it, so a node that installs a snapshot still recognizes a resent write.
    pub sessions: BTreeMap<u64, ClientSession>,
}

impl StateMachineData {
    /// Apply `command`, committed at log index `version`, and return its response.
    ///
    /// A command with a [`types_kv::RequestId`] is applied at most once. When its serial equals the
    /// latest applied one of its client, it is a resent copy of that write, and gets the stored
    /// response of that write. A serial below it, which a client that follows the rule of
    /// `RequestId` never sends, is not applied, and its response holds no value.
    #[since(version = "0.10.2")]
    pub fn apply<D>(&mut self, command: &D, version: u64) -> types_kv::Response
    where D: KvCommand {
        let Some(id) = command.request_id() else {
            return self.apply_request(command.request(), version);
        };

        if let Some(session) = self.sessions.get(&id.client_id) {
            match id.serial.cmp(&session.serial) {
                Ordering::Equal => return session.response.clone(),
                Ordering::Less => {
                    tracing::warn!(%id, latest = session.serial, "skip a write older than the latest of its client");
                    return types_kv::Response::none();
                }
                Ordering::Greater => {}
            }
        }

        let response = self.apply_request(command.request(), version);
        let session = ClientSession {
            serial: id.serial,
            response: response.clone(),
        };
        self.sessions.insert(id.client_id, session);
        response
    }

    fn apply_request(&mut self, request: &types_kv::Request, version: u64) -> types_kv::Response {
        match request {
            types_kv::Request::Set { key, value } => {
                self.data.insert(key.clone(), types_kv::VersionedValue {
                    value: value.clone(),
                    version,
                });
                types_kv::Response::new(value.clone(), version)
            }
            types_kv::Request::CompareAndSet {
                key,
                expected_version,
                value,
            } => {
                let matches = self.data.get(key).is_some_and(|current| current.version == *expected_version);

                if matches {
                    self.data.insert(key.clone(), types_kv::VersionedValue {
                        value: value.clone(),
                        version,
                    });
                    types_kv::Response::new(value.clone(), version)
                } else {
                    types_kv::Response::none()
                }
            }
        }
    }
}

/// Inner storage for the state machine.
#[derive(Debug)]
pub struct StateMachineStoreInner<C: RaftTypeConfig> {
    pub last_applied_log: Option<LogIdOf<C>>,

    pub last_membership: StoredMembershipOf<C>,

    /// The Raft state machine.
    pub state_machine: StateMachineData,

    /// The last received snapshot.
    pub current_snapshot: Option<StoredSnapshot<C>>,
}

impl<C: RaftTypeConfig> Default for StateMachineStoreInner<C> {
    fn default() -> Self {
        Self {
            last_applied_log: None,
            last_membership: StoredMembershipOf::<C>::default(),
            state_machine: StateMachineData::default(),
            current_snapshot: None,
        }
    }
}

/// Defines a state machine for the Raft cluster.
///
/// This is a new type wrapper around `Arc<Mutex<StateMachineStoreInner<C>>>` to satisfy
/// Rust's orphan rules when implementing foreign traits.
#[derive(Clone, Debug)]
pub struct StateMachineStore<C: RaftTypeConfig>(Arc<Mutex<StateMachineStoreInner<C>>>);

impl<C: RaftTypeConfig> Default for StateMachineStore<C> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(StateMachineStoreInner::default())))
    }
}

impl<C: RaftTypeConfig> StateMachineStore<C> {
    /// Get a value from the state machine by key, locking the state machine.
    pub async fn get(&self, key: &str) -> Option<types_kv::VersionedValue> {
        let inner = self.0.lock().await;
        inner.state_machine.data.get(key).cloned()
    }
}

impl<C> RaftSnapshotBuilder<C> for StateMachineStore<C>
where
    C: RaftTypeConfig<R = types_kv::Response, Payload = EntryPayloadOf<C>, Entry = DefaultEntryOf<C>>,
    C::D: KvCommand,
{
    type SnapshotData = Cursor<Vec<u8>>;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, Cursor<Vec<u8>>>, io::Error> {
        let mut inner = self.0.lock().await;

        // The sessions go into the snapshot with the data: a node that installs it must still recognize a
        // resent write.
        let data =
            serde_json::to_vec(&inner.state_machine).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let meta = SnapshotMetaOf::<C> {
            last_log_id: inner.last_applied_log.clone(),
            last_membership: inner.last_membership.clone(),
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        inner.current_snapshot = Some(snapshot);

        Ok(SnapshotOf::<C, Cursor<Vec<u8>>> {
            meta,
            snapshot: Cursor::new(data),
        })
    }
}

impl<C> RaftStateMachine<C> for StateMachineStore<C>
where
    C: RaftTypeConfig<R = types_kv::Response, Payload = EntryPayloadOf<C>, Entry = DefaultEntryOf<C>>,
    C::D: KvCommand,
{
    type SnapshotData = Cursor<Vec<u8>>;

    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        let inner = self.0.lock().await;
        Ok((inner.last_applied_log.clone(), inner.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        let mut inner = self.0.lock().await;

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "replicate to sm");

            let version = entry.log_id.index();
            inner.last_applied_log = Some(entry.log_id.clone());

            let response = match &entry.payload {
                EntryPayload::Blank => types_kv::Response::none(),
                EntryPayload::Normal(command) => inner.state_machine.apply(command, version),
                EntryPayload::Membership(mem) => {
                    inner.last_membership = StoredMembershipOf::<C>::new(Some(entry.log_id.clone()), mem.clone());
                    types_kv::Response::none()
                }
            };

            if let Some(responder) = responder {
                responder.send(response);
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<C>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        let state_machine: StateMachineData =
            serde_json::from_slice(&new_snapshot.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut inner = self.0.lock().await;
        inner.last_applied_log = meta.last_log_id.clone();
        inner.last_membership = meta.last_membership.clone();
        inner.state_machine = state_machine;
        inner.current_snapshot = Some(new_snapshot);

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, Self::SnapshotData>>, io::Error> {
        let inner = self.0.lock().await;
        match &inner.current_snapshot {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(SnapshotOf::<C, Self::SnapshotData> {
                    meta: snapshot.meta.clone(),
                    snapshot: Cursor::new(data),
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

/// Supports snapshot reception through the legacy v1 network protocol.
impl<C> SnapshotReceiverFactory<C> for StateMachineStore<C>
where
    C: RaftTypeConfig<R = types_kv::Response, Payload = EntryPayloadOf<C>, Entry = DefaultEntryOf<C>>,
    C::D: KvCommand,
{
    type SnapshotReceiver = Cursor<Vec<u8>>;

    async fn begin_receiving_snapshot(&mut self) -> Result<Self::SnapshotReceiver, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use openraft::entry::RaftEntry;
    use openraft::testing::log_id;
    use types_kv::DedupRequest;
    use types_kv::Request;
    use types_kv::RequestId;
    use types_kv::Response;

    use super::*;

    openraft::declare_raft_types!(DedupConfig: D = DedupRequest, R = Response);

    fn write(client_id: u64, serial: u64, value: &str) -> DedupRequest {
        DedupRequest::new(RequestId::new(client_id, serial), Request::set("key", value))
    }

    #[test]
    fn a_resent_write_gets_its_first_response_and_does_not_apply_again() {
        let mut sm = StateMachineData::default();
        assert_eq!(Response::new("a", 5), sm.apply(&write(7, 1, "a"), 5));
        assert_eq!(Response::new("a", 5), sm.apply(&write(7, 1, "a"), 6));
        assert_eq!(Response::new("a", 5).value, sm.data.get("key").cloned());
    }

    #[test]
    fn a_later_write_of_the_client_applies() {
        let mut sm = StateMachineData::default();
        sm.apply(&write(7, 1, "a"), 5);
        assert_eq!(Response::new("b", 6), sm.apply(&write(7, 2, "b"), 6));
        assert_eq!(Response::new("b", 6).value, sm.data.get("key").cloned());
    }

    #[test]
    fn an_earlier_write_sent_after_a_later_one_does_not_apply() {
        let mut sm = StateMachineData::default();
        sm.apply(&write(7, 2, "b"), 5);
        assert_eq!(Response::none(), sm.apply(&write(7, 1, "a"), 6));
        assert_eq!(Response::new("b", 5).value, sm.data.get("key").cloned());
    }

    #[test]
    fn each_client_has_its_own_session() {
        let mut sm = StateMachineData::default();
        sm.apply(&write(7, 1, "a"), 5);
        assert_eq!(Response::new("b", 6), sm.apply(&write(8, 1, "b"), 6));
        assert_eq!(Response::new("a", 5), sm.apply(&write(7, 1, "a"), 7));
    }

    #[test]
    fn a_request_without_an_id_applies_each_time_it_is_committed() {
        let mut sm = StateMachineData::default();
        let request = Request::set("key", "a");
        assert_eq!(Response::new("a", 5), sm.apply(&request, 5));
        assert_eq!(Response::new("a", 6), sm.apply(&request, 6));
        assert!(sm.sessions.is_empty());
    }

    #[test]
    fn a_node_that_installs_a_snapshot_recognizes_a_resent_write() {
        block_on(async {
            let mut leader = StateMachineStore::<DedupConfig>::default();
            let entry = RaftEntry::new_normal(log_id::<DedupConfig>(1, 1, 5), write(7, 1, "a"));
            let entries = futures::stream::iter([Ok::<_, io::Error>((entry, None))]);
            leader.apply(entries).await.unwrap();
            let snapshot = leader.get_snapshot_builder().await.build_snapshot().await.unwrap();

            let mut follower = StateMachineStore::<DedupConfig>::default();
            follower.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();
            let resent = RaftEntry::new_normal(log_id::<DedupConfig>(2, 2, 6), write(7, 1, "a"));
            let entries = futures::stream::iter([Ok::<_, io::Error>((resent, None))]);
            follower.apply(entries).await.unwrap();

            assert_eq!(Response::new("a", 5).value, follower.get("key").await);
            let inner = follower.0.lock().await;
            let session = ClientSession {
                serial: 1,
                response: Response::new("a", 5),
            };
            assert_eq!(Some(&session), inner.state_machine.sessions.get(&7));
        });
    }
}
