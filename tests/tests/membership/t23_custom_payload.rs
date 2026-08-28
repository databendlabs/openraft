use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::Stream;
use futures::TryStreamExt;
use maplit::btreemap;
use maplit::btreeset;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::Entry;
use openraft::LogState;
use openraft::Membership;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::RaftLogReader;
use openraft::RaftNetworkFactory;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::Snapshot;
use openraft::StoredMembership;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftPayload;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ChangeMembershipRequest;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use serde::Deserialize;
use serde::Serialize;

use crate::fixtures::ut_harness;

static BLANK_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
struct CustomPayload {
    normal: Option<String>,
    membership: Option<Membership<u64, ()>>,
}

impl fmt::Display for CustomPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "normal={:?}, membership={:?}", self.normal, self.membership)
    }
}

impl RaftPayload for CustomPayload {
    type D = String;
    type NodeId = u64;
    type Node = ();

    fn blank() -> Self {
        BLANK_CALLS.fetch_add(1, Ordering::SeqCst);
        Self::default()
    }

    fn with_normal(mut self, data: String) -> Self {
        self.normal = Some(data);
        self
    }

    fn with_membership(mut self, membership: Membership<u64, ()>) -> Self {
        self.membership = Some(membership);
        self
    }

    fn get_membership(&self) -> Option<Membership<u64, ()>> {
        self.membership.clone()
    }
}

openraft::declare_raft_types!(
    TestConfig:
        D = String,
        R = (),
        Node = (),
        Payload = CustomPayload,
);

#[derive(Clone, Debug, Default)]
struct TestLogStore(Arc<Mutex<TestLogStoreInner>>);

#[derive(Debug, Default)]
struct TestLogStoreInner {
    last_purged_log_id: Option<LogIdOf<TestConfig>>,
    log: BTreeMap<u64, EntryOf<TestConfig>>,
    committed: Option<LogIdOf<TestConfig>>,
    vote: Option<VoteOf<TestConfig>>,
}

impl RaftLogReader<TestConfig> for TestLogStore {
    async fn try_get_log_entries<RB>(&mut self, range: RB) -> Result<Vec<EntryOf<TestConfig>>, io::Error>
    where RB: RangeBounds<u64> + Clone + fmt::Debug + OptionalSend {
        let inner = self.0.lock().unwrap();
        let entries = inner.log.range(range).map(|(_, entry)| entry.clone()).collect();
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<TestConfig>>, io::Error> {
        let inner = self.0.lock().unwrap();
        Ok(inner.vote)
    }
}

impl RaftLogStorage<TestConfig> for TestLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TestConfig>, io::Error> {
        let inner = self.0.lock().unwrap();
        let last_log_id = inner.log.last_key_value().map(|(_, entry)| entry.log_id);
        let last_log_id = last_log_id.or(inner.last_purged_log_id);

        Ok(LogState {
            last_purged_log_id: inner.last_purged_log_id,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<TestConfig>) -> Result<(), io::Error> {
        self.0.lock().unwrap().vote = Some(*vote);
        Ok(())
    }

    async fn save_committed(&mut self, committed: Option<LogIdOf<TestConfig>>) -> Result<(), io::Error> {
        self.0.lock().unwrap().committed = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogIdOf<TestConfig>>, io::Error> {
        let inner = self.0.lock().unwrap();
        Ok(inner.committed)
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TestConfig>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<TestConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut inner = self.0.lock().unwrap();
        for entry in entries {
            inner.log.insert(entry.log_id.index, entry);
        }
        drop(inner);

        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<TestConfig>>) -> Result<(), io::Error> {
        let start_index = last_log_id.map(|log_id| log_id.index + 1).unwrap_or(0);
        self.0.lock().unwrap().log.retain(|index, _| *index < start_index);
        Ok(())
    }

    async fn purge(&mut self, log_id: LogIdOf<TestConfig>) -> Result<(), io::Error> {
        let mut inner = self.0.lock().unwrap();
        assert!(inner.last_purged_log_id.as_ref() <= Some(&log_id));
        inner.last_purged_log_id = Some(log_id);
        inner.log.retain(|index, _| *index > log_id.index);
        Ok(())
    }
}

type TestRaft = Raft<TestConfig, TestStateMachine>;

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
struct AppliedState {
    last_applied: Option<LogIdOf<TestConfig>>,
    membership: StoredMembershipOf<TestConfig>,
    commands: Vec<(LogIdOf<TestConfig>, String)>,
}

#[derive(Clone, Debug)]
struct StoredSnapshot {
    meta: SnapshotMetaOf<TestConfig>,
    data: Vec<u8>,
}

#[derive(Debug, Default)]
struct StateMachineInner {
    state: AppliedState,
    snapshot: Option<StoredSnapshot>,
}

#[derive(Clone, Debug, Default)]
struct TestStateMachine(Arc<Mutex<StateMachineInner>>);

impl TestStateMachine {
    fn state(&self) -> AppliedState {
        self.0.lock().unwrap().state.clone()
    }

    fn apply_entry(&self, entry: EntryOf<TestConfig>) {
        let log_id = entry.log_id;
        let payload = entry.payload;
        let mut inner = self.0.lock().unwrap();

        inner.state.last_applied = Some(log_id);
        if let Some(membership) = payload.membership {
            inner.state.membership = StoredMembership::new(Some(log_id), membership);
        }
        if let Some(normal) = payload.normal {
            inner.state.commands.push((log_id, normal));
        }
    }
}

impl RaftSnapshotBuilder<TestConfig> for TestStateMachine {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<TestConfig, Self::SnapshotData>, io::Error> {
        let mut inner = self.0.lock().unwrap();
        let data = bincode::serialize(&inner.state).map_err(invalid_data)?;
        let meta = SnapshotMetaOf::<TestConfig> {
            last_log_id: inner.state.last_applied,
            last_membership: inner.state.membership.clone(),
        };

        inner.snapshot = Some(StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        });

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(data),
        })
    }
}

impl RaftStateMachine<TestConfig> for TestStateMachine {
    type SnapshotData = Cursor<Vec<u8>>;
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogIdOf<TestConfig>>, StoredMembershipOf<TestConfig>), io::Error> {
        let state = self.state();
        Ok((state.last_applied, state.membership))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TestConfig>, io::Error>> + Unpin + OptionalSend {
        while let Some((entry, responder)) = entries.try_next().await? {
            self.apply_entry(entry);
            if let Some(responder) = responder {
                responder.send(());
            }
        }

        Ok(())
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<TestConfig>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        let data = snapshot.into_inner();
        let mut state: AppliedState = bincode::deserialize(&data).map_err(invalid_data)?;
        state.last_applied = meta.last_log_id;
        state.membership = meta.last_membership.clone();

        let mut inner = self.0.lock().unwrap();
        inner.state = state;
        inner.snapshot = Some(StoredSnapshot {
            meta: meta.clone(),
            data,
        });
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<TestConfig, Self::SnapshotData>>, io::Error> {
        let inner = self.0.lock().unwrap();
        let snapshot = inner.snapshot.as_ref().map(|stored| Snapshot {
            meta: stored.meta.clone(),
            snapshot: Cursor::new(stored.data.clone()),
        });
        Ok(snapshot)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

fn invalid_data(error: bincode::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[derive(Clone, Default)]
struct Router {
    nodes: Arc<Mutex<BTreeMap<u64, TestRaft>>>,
}

impl Router {
    fn insert(&self, node_id: u64, raft: TestRaft) {
        self.nodes.lock().unwrap().insert(node_id, raft);
    }

    fn remove(&self, node_id: u64) -> Option<TestRaft> {
        self.nodes.lock().unwrap().remove(&node_id)
    }

    fn get(&self, node_id: u64) -> Option<TestRaft> {
        self.nodes.lock().unwrap().get(&node_id).cloned()
    }
}

impl RaftNetworkFactory<TestConfig> for Router {
    type Network = Connection;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        Connection {
            router: self.clone(),
            target,
        }
    }
}

struct Connection {
    router: Router,
    target: u64,
}

impl Connection {
    fn target(&self) -> Result<TestRaft, RPCError<TestConfig>> {
        self.router.get(self.target).ok_or_else(|| {
            let error = Unreachable::from_string(format!("node {} is not running", self.target));
            RPCError::Unreachable(error)
        })
    }
}

impl RaftNetworkV2<TestConfig> for Connection {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<TestConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TestConfig>, RPCError<TestConfig>> {
        let target = self.target()?;
        let response = target.append_entries(request).await;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn full_snapshot(
        &mut self,
        vote: <TestConfig as RaftTypeConfig>::Vote,
        snapshot: SnapshotOf<TestConfig, Self::SnapshotData>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<TestConfig>, StreamingError<TestConfig>> {
        let target = self.target()?;
        let response = target.install_full_snapshot(vote, snapshot).await;
        let response = response.map_err(|error| Unreachable::new(&error))?;
        Ok(response)
    }

    async fn vote(
        &mut self,
        request: VoteRequest<TestConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TestConfig>, RPCError<TestConfig>> {
        let target = self.target()?;
        let response = target.vote(request).await;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }
}

async fn new_node(
    node_id: u64,
    config: Arc<Config>,
    router: Router,
    log_store: TestLogStore,
    state_machine: TestStateMachine,
) -> anyhow::Result<TestRaft> {
    let raft = Raft::new(node_id, config, router, log_store, state_machine).await?;
    Ok(raft)
}

async fn read_payloads(
    log_store: &TestLogStore,
    range: RangeInclusive<u64>,
) -> anyhow::Result<Vec<(LogIdOf<TestConfig>, CustomPayload)>> {
    let mut reader = log_store.clone();
    let entries: Vec<Entry<_, CustomPayload>> = reader.try_get_log_entries(range).await?;
    let payloads = entries.into_iter().map(|entry| (entry.log_id, entry.payload)).collect();
    Ok(payloads)
}

/// A custom payload carries distinct application data through both membership entries and recovery.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn custom_payload_survives_membership_change_and_recovery() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Router::default();
    let log0 = TestLogStore::default();
    let log1 = TestLogStore::default();
    let sm0 = TestStateMachine::default();
    let sm1 = TestStateMachine::default();

    let node0 = new_node(0, config.clone(), router.clone(), log0.clone(), sm0.clone()).await?;
    let node1 = new_node(1, config.clone(), router.clone(), log1.clone(), sm1.clone()).await?;
    router.insert(0, node0.clone());
    router.insert(1, node1.clone());

    tracing::info!("--- initialize node 0 and add node 1 as a learner");
    {
        node0.initialize(btreeset! {0}).await?;
        node0.wait(timeout()).applied_index(Some(1), "initial leader blank").await?;

        let response = node0.add_learner(1, (), true).await?;
        assert_eq!(2, response.log_id.index);
        node1.wait(timeout()).applied_index(Some(2), "learner applied membership").await?;
    }

    let joint_membership = Membership::new_with_defaults(vec![btreeset! {0}, btreeset! {0,1}], []);
    let uniform_membership = Membership::new_with_defaults(vec![btreeset! {0,1}], []);

    tracing::info!("--- promote node 1 with distinct payloads for the joint and uniform entries");
    let (joint_log_id, uniform_log_id) = {
        let joint_payload = CustomPayload::normal("joint application data".to_string());
        let uniform_payload = CustomPayload::normal("uniform application data".to_string());
        let request = ChangeMembershipRequest::new([0, 1], false).with_payload(joint_payload, uniform_payload);
        let outcome = node0.change_membership_with_payload(request).await?;
        let joint = outcome.joint.expect("voter change should enter joint consensus");

        assert_eq!(Some(joint_membership.clone()), joint.membership);
        assert_eq!(Some(uniform_membership.clone()), outcome.uniform.membership);
        assert_eq!(3, joint.log_id.index);
        assert_eq!(4, outcome.uniform.log_id.index);
        assert!(joint.log_id < outcome.uniform.log_id);

        (joint.log_id, outcome.uniform.log_id)
    };

    tracing::info!("--- both nodes store and apply both custom membership payloads");
    {
        node0.wait(timeout()).applied_index(Some(4), "leader applied uniform membership").await?;
        node1.wait(timeout()).applied_index(Some(4), "follower applied uniform membership").await?;

        let expected_payloads = vec![
            (joint_log_id, CustomPayload {
                normal: Some("joint application data".to_string()),
                membership: Some(joint_membership.clone()),
            }),
            (uniform_log_id, CustomPayload {
                normal: Some("uniform application data".to_string()),
                membership: Some(uniform_membership.clone()),
            }),
        ];
        assert_eq!(expected_payloads, read_payloads(&log0, 3..=4).await?);
        assert_eq!(expected_payloads, read_payloads(&log1, 3..=4).await?);

        let expected_state = AppliedState {
            last_applied: Some(uniform_log_id),
            membership: StoredMembership::new(Some(uniform_log_id), uniform_membership.clone()),
            commands: vec![
                (joint_log_id, "joint application data".to_string()),
                (uniform_log_id, "uniform application data".to_string()),
            ],
        };
        assert_eq!(expected_state, sm0.state());
        assert_eq!(expected_state, sm1.state());

        let leader_metrics = node0.metrics().borrow_watched().clone();
        assert_eq!(&expected_state.membership, leader_metrics.membership_config.as_ref());
        let follower_metrics = node1.metrics().borrow_watched().clone();
        assert_eq!(&expected_state.membership, follower_metrics.membership_config.as_ref());
    }

    let final_joint_membership = Membership::new_with_defaults(vec![btreeset! {0,1}, btreeset! {0}], []);
    let final_membership = Membership::new_with_defaults(vec![btreeset! {0}], []);

    tracing::info!("--- a request without payload creates a separate blank for each membership entry");
    {
        let blank_calls = BLANK_CALLS.load(Ordering::SeqCst);
        let request = ChangeMembershipRequest::new([0], false);
        let outcome = node0.change_membership_with_payload(request).await?;
        let joint = outcome.joint.expect("voter change should enter joint consensus");
        let new_blank_calls = BLANK_CALLS.load(Ordering::SeqCst);

        assert_eq!(blank_calls + 2, new_blank_calls);
        assert_eq!(5, joint.log_id.index);
        assert_eq!(6, outcome.uniform.log_id.index);
        assert_eq!(Some(final_joint_membership.clone()), joint.membership);
        assert_eq!(Some(final_membership.clone()), outcome.uniform.membership);

        let expected_payloads = vec![
            (joint.log_id, CustomPayload {
                normal: None,
                membership: Some(final_joint_membership.clone()),
            }),
            (outcome.uniform.log_id, CustomPayload {
                normal: None,
                membership: Some(final_membership.clone()),
            }),
        ];
        assert_eq!(expected_payloads, read_payloads(&log0, 5..=6).await?);
    }

    // Voters `{0}` unchanged, node 1 back as a learner: one voter set on both sides, so this is a
    // direct append.
    let learner_membership = Membership::new(vec![btreeset! {0}], btreemap! {0=>(), 1=>()})?;

    tracing::info!("--- a change that moves no voter writes one uniform entry, with the uniform payload");
    let learner_log_id = {
        let joint_payload = CustomPayload::normal("joint application data".to_string());
        let uniform_payload = CustomPayload::normal("learner application data".to_string());
        let changes = ChangeMembers::AddNodes(btreemap! {1 => ()});
        let request = ChangeMembershipRequest::new(changes, false).with_payload(joint_payload, uniform_payload);
        let outcome = node0.change_membership_with_payload(request).await?;

        assert!(outcome.joint.is_none(), "adding a learner needs no joint membership");
        assert_eq!(7, outcome.uniform.log_id.index);
        assert_eq!(Some(learner_membership.clone()), outcome.uniform.membership);

        let expected_payloads = vec![(outcome.uniform.log_id, CustomPayload {
            normal: Some("learner application data".to_string()),
            membership: Some(learner_membership.clone()),
        })];
        assert_eq!(expected_payloads, read_payloads(&log0, 7..=7).await?);

        outcome.uniform.log_id
    };

    tracing::info!("--- append_membership binds the membership into the caller's payload");
    let appended_log_id = {
        let blank_calls = BLANK_CALLS.load(Ordering::SeqCst);

        // The base payload already carries a membership; `with_membership()` replaces exactly that
        // field and keeps `normal`.
        let base = CustomPayload {
            normal: Some("appended application data".to_string()),
            membership: Some(final_joint_membership.clone()),
        };
        let resp = node0.append_membership(learner_membership.clone(), base, []).await?;

        assert_eq!(blank_calls, BLANK_CALLS.load(Ordering::SeqCst));
        assert_eq!(8, resp.log_id.index);
        assert_eq!(Some(learner_membership.clone()), resp.membership);

        // One entry holds both the application data and the proposed membership.
        let expected_payloads = vec![(resp.log_id, CustomPayload {
            normal: Some("appended application data".to_string()),
            membership: Some(learner_membership.clone()),
        })];
        assert_eq!(expected_payloads, read_payloads(&log0, 8..=8).await?);

        resp.log_id
    };

    let expected_final_state = AppliedState {
        last_applied: Some(appended_log_id),
        membership: StoredMembership::new(Some(appended_log_id), learner_membership),
        commands: vec![
            (joint_log_id, "joint application data".to_string()),
            (uniform_log_id, "uniform application data".to_string()),
            (learner_log_id, "learner application data".to_string()),
            (appended_log_id, "appended application data".to_string()),
        ],
    };
    assert_eq!(expected_final_state, sm0.state());

    tracing::info!("--- restart node 0 from its log with an empty state machine");
    let recovered_node = {
        let removed = router.remove(0);
        assert!(removed.is_some());
        node0.shutdown().await?;

        let recovered_sm = TestStateMachine::default();
        let recovered = new_node(0, config.clone(), router.clone(), log0, recovered_sm.clone()).await?;
        router.insert(0, recovered.clone());
        recovered.wait(timeout()).applied_index(Some(8), "reapply committed log").await?;

        assert_eq!(expected_final_state, recovered_sm.state());
        let metrics = recovered.metrics().borrow_watched().clone();
        assert_eq!(&expected_final_state.membership, metrics.membership_config.as_ref());
        recovered
    };

    tracing::info!("--- install node 0 snapshot into a fresh node");
    let node2 = {
        recovered_node.trigger().snapshot().await?;
        recovered_node.wait(timeout()).snapshot(appended_log_id, "custom payload snapshot").await?;
        let snapshot = recovered_node.get_snapshot().await?.expect("snapshot should exist");
        let vote = recovered_node.metrics().borrow_watched().vote;

        let snapshot_sm = TestStateMachine::default();
        let node = new_node(2, config, router.clone(), TestLogStore::default(), snapshot_sm.clone()).await?;
        router.insert(2, node.clone());
        node.install_full_snapshot(vote, snapshot).await?;
        // `install_full_snapshot()` answers as soon as the snapshot is installed, before the
        // RaftCore loop publishes the next metrics. Wait for the installed index, so that the
        // membership assertion below reads the metrics the snapshot produced.
        node.wait(timeout()).applied_index(Some(appended_log_id.index), "install snapshot").await?;

        assert_eq!(expected_final_state, snapshot_sm.state());
        let metrics = node.metrics().borrow_watched().clone();
        assert_eq!(&expected_final_state.membership, metrics.membership_config.as_ref());
        node
    };

    recovered_node.shutdown().await?;
    node1.shutdown().await?;
    node2.shutdown().await?;
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(2))
}
