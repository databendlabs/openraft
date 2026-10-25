//! A Raft cluster for type configs whose membership is not the default `Membership`.
//!
//! [`RaftRouter`](crate::fixtures::RaftRouter) runs only the memstore type config, so it cannot
//! run another membership type. [`TestCluster`] runs any [`TestConfig`].

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_set;
use std::fmt;
use std::future::Future;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use futures::Stream;
use futures::TryStreamExt;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::RaftLogReader;
use openraft::RaftNetworkFactory;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::alias::VoteOf;
use openraft::entry::RaftEntry;
use openraft::errors::MembershipError;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::QuorumBridge;
use openraft::membership::QuorumIntersection;
use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::storage::RaftStateMachine;
use openraft::testing::log_id;
use serde::Deserialize;
use serde::Serialize;

/// A quorum rule that accepts a set of voters when the set contains one of the listed quorums.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalQuorums(pub Vec<BTreeSet<u64>>);

impl QuorumSet for MinimalQuorums {
    type Id = u64;
    type Iter = btree_set::IntoIter<u64>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where I: Iterator<Item = &'a u64> + Clone {
        let ids: BTreeSet<u64> = ids.copied().collect();
        self.0.iter().any(|quorum| quorum.is_subset(&ids))
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for quorum in &self.0 {
            ids.extend(quorum);
        }
        ids.into_iter()
    }
}

impl QuorumIntersection<MinimalQuorums> for MinimalQuorums {
    /// Every quorum contains a listed quorum, so two rules intersect exactly when each listed
    /// quorum of one meets each listed quorum of the other.
    fn intersects_with(&self, other: &MinimalQuorums) -> Option<bool> {
        for quorum in &self.0 {
            for other_quorum in &other.0 {
                if quorum.is_disjoint(other_quorum) {
                    return Some(false);
                }
            }
        }
        Some(true)
    }
}

impl QuorumBridge<MinimalQuorums> for MinimalQuorums {
    fn joint_with(&self, _other: MinimalQuorums) -> Self {
        unimplemented!("the test never changes the membership")
    }
}

/// A membership whose read and write quorum rules differ, with configurable node roles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalQuorumsMembership {
    pub read: MinimalQuorums,
    pub write: MinimalQuorums,
    /// Explicit roles and extra known nodes. Voters default to `Campaigner | ReceiveLogs`.
    pub roles: BTreeMap<u64, NodeRole>,
}

impl fmt::Display for MinimalQuorumsMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{read: {:?}, write: {:?}, roles: {:?}}}",
            self.read.0, self.write.0, self.roles
        )
    }
}

impl RaftMembership for MinimalQuorumsMembership {
    type NodeId = u64;
    type Node = ();
    type ErrorSource = openraft::impls::BoxedErrorSource;
    type QuorumSet = MinimalQuorums;

    fn read_quorum_set(&self) -> &MinimalQuorums {
        &self.read
    }

    fn write_quorum_set(&self) -> &MinimalQuorums {
        &self.write
    }

    fn get_node(&self, node_id: &u64) -> Option<NodeRef<'_, ()>> {
        let is_read_voter = self.read.ids().any(|id| id == *node_id);
        let is_write_voter = self.write.ids().any(|id| id == *node_id);
        let role = if let Some(role) = self.roles.get(node_id) {
            *role
        } else if is_read_voter || is_write_voter {
            NodeRole::Campaigner | NodeRole::ReceiveLogs
        } else {
            return None;
        };

        let node_ref = NodeRef {
            node: &(),
            role,
            is_read_voter,
            is_write_voter,
        };
        Some(node_ref)
    }

    type Iter = btree_set::IntoIter<u64>;

    fn node_ids(&self) -> Self::Iter {
        let mut ids: BTreeSet<u64> = self.read.ids().collect();
        ids.extend(self.write.ids());
        ids.extend(self.roles.keys());
        ids.into_iter()
    }

    fn new<I>(_read_quorum_set: MinimalQuorums, _write_quorum_set: MinimalQuorums, _nodes: I) -> Self
    where I: IntoIterator<Item = (u64, (), NodeRole)> {
        unimplemented!("the test never changes the membership")
    }
}

impl RaftMembershipEdit<MinimalQuorumsMembership> for ChangeMembers<u64, ()> {
    fn apply(
        self,
        _membership: &MinimalQuorumsMembership,
    ) -> Result<MinimalQuorumsMembership, MembershipError<u64, openraft::impls::BoxedErrorSource>> {
        unimplemented!("the test never changes the membership")
    }

    fn add_learner(node_id: u64, node: ()) -> Self {
        let nodes = BTreeMap::from([(node_id, node)]);
        ChangeMembers::AddNodes(nodes)
    }

    fn make_uniform() -> Self {
        ChangeMembers::AddVoterIds(BTreeSet::new())
    }
}

/// A type config that [`TestCluster`] can run.
pub trait TestConfig
where Self: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = ()>
{
}

impl<C> TestConfig for C where C: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = ()> {}

#[derive(Clone, Debug, Default)]
pub struct LogStore<C>(Arc<Mutex<LogStoreInner<C>>>)
where C: TestConfig;

#[derive(Debug, Default)]
pub struct LogStoreInner<C>
where C: TestConfig
{
    log: BTreeMap<u64, EntryOf<C>>,
    vote: Option<VoteOf<C>>,
}

impl<C> RaftLogReader<C> for LogStore<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    async fn try_get_log_entries<RB>(&mut self, range: RB) -> Result<Vec<EntryOf<C>>, io::Error>
    where RB: RangeBounds<u64> + Clone + fmt::Debug + OptionalSend {
        let inner = self.0.lock().unwrap();
        let range_entries = inner.log.range(range);
        let entries = range_entries.map(|(_, entry)| entry.clone()).collect();
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, io::Error> {
        let inner = self.0.lock().unwrap();
        Ok(inner.vote.clone())
    }
}

impl<C> RaftLogStorage<C> for LogStore<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<C>, io::Error> {
        let inner = self.0.lock().unwrap();
        let last_entry = inner.log.last_key_value();
        let last_log_id = last_entry.map(|(_, entry)| entry.log_id());

        Ok(LogState {
            last_purged_log_id: None,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), io::Error> {
        let mut inner = self.0.lock().unwrap();
        inner.vote = Some(vote.clone());
        Ok(())
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<C>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut inner = self.0.lock().unwrap();
        for entry in entries {
            inner.log.insert(entry.index(), entry);
        }
        drop(inner);

        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        let start_index = last_log_id.next_index();
        let mut inner = self.0.lock().unwrap();
        inner.log.retain(|index, _| *index < start_index);
        Ok(())
    }

    async fn purge(&mut self, _log_id: LogIdOf<C>) -> Result<(), io::Error> {
        unimplemented!("the test builds no snapshot, so it purges no log")
    }
}

/// A state machine that keeps every entry it applies.
#[derive(Clone, Debug, Default)]
pub struct StateMachine<C>(Arc<Mutex<StateMachineInner<C>>>)
where C: TestConfig;

#[derive(Debug, Default)]
pub struct StateMachineInner<C>
where C: TestConfig
{
    applied: Vec<EntryOf<C>>,
    membership: StoredMembershipOf<C>,
}

impl<C> StateMachine<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    fn applied(&self) -> Vec<EntryOf<C>> {
        let inner = self.0.lock().unwrap();
        inner.applied.clone()
    }
}

impl<C> RaftSnapshotBuilder<C> for StateMachine<C>
where C: TestConfig
{
    type SnapshotData = ();

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, ()>, io::Error> {
        unimplemented!("the test builds no snapshot")
    }
}

impl<C> RaftStateMachine<C> for StateMachine<C>
where C: TestConfig
{
    type SnapshotData = ();
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        let inner = self.0.lock().unwrap();
        let last_entry = inner.applied.last();
        let last_applied = last_entry.map(|entry| entry.log_id());
        Ok((last_applied, inner.membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        while let Some((entry, responder)) = entries.try_next().await? {
            {
                let mut inner = self.0.lock().unwrap();
                if let Some(membership) = entry.get_membership() {
                    let log_id = entry.log_id();
                    inner.membership = StoredMembershipOf::<C>::new(Some(log_id), membership);
                }
                inner.applied.push(entry);
            }

            if let Some(responder) = responder {
                responder.send(());
            }
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn install_snapshot(&mut self, _meta: &SnapshotMetaOf<C>, _snapshot: ()) -> Result<(), io::Error> {
        unimplemented!("the test sends no snapshot")
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, ()>>, io::Error> {
        Ok(None)
    }
}

pub type TestRaft<C> = Raft<C, StateMachine<C>>;

/// Delivers RPCs among the nodes of one test, except between the two nodes of a cut link.
pub struct Router<C>
where C: TestConfig
{
    nodes: BTreeMap<u64, TestRaft<C>>,
    cut_links: BTreeSet<(u64, u64)>,
}

/// The network that node `source` sends its RPCs through.
pub struct Network<C>
where C: TestConfig
{
    source: u64,
    router: Arc<Mutex<Router<C>>>,
}

impl<C> RaftNetworkFactory<C> for Network<C>
where C: TestConfig
{
    type Network = Connection<C>;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        Connection {
            source: self.source,
            target,
            router: self.router.clone(),
        }
    }
}

pub struct Connection<C>
where C: TestConfig
{
    source: u64,
    target: u64,
    router: Arc<Mutex<Router<C>>>,
}

impl<C> Connection<C>
where C: TestConfig
{
    /// Returns the target node, or `Unreachable` if the link to it is cut.
    fn target(&self) -> Result<TestRaft<C>, RPCError<C>> {
        let router = self.router.lock().unwrap();
        let link = (self.source, self.target);
        let reversed = (self.target, self.source);
        let is_cut = router.cut_links.contains(&link) || router.cut_links.contains(&reversed);
        if is_cut {
            let error = Unreachable::from_string(format!("link {}-{} is cut", self.source, self.target));
            return Err(RPCError::Unreachable(error));
        }

        let Some(raft) = router.nodes.get(&self.target) else {
            let error = Unreachable::from_string(format!("node {} is not running", self.target));
            return Err(RPCError::Unreachable(error));
        };
        Ok(raft.clone())
    }
}

impl<C> RaftNetworkV2<C> for Connection<C>
where C: TestConfig
{
    type SnapshotData = ();

    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        let target = self.target()?;
        let response = target.append_entries(request).await;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn vote(&mut self, request: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let target = self.target()?;
        let response = target.vote(request).await;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn pre_vote(&mut self, request: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let target = self.target()?;
        let response = target.pre_vote(request).await;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn transfer_leader(
        &mut self,
        request: TransferLeaderRequest<C>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<C>, RPCError<C>> {
        let target = self.target()?;
        let response = target.handle_transfer_leader(request).await;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, ()>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        unimplemented!("the test sends no snapshot")
    }
}

/// Nodes of one membership, connected through one router.
pub struct TestCluster<C>
where C: TestConfig
{
    router: Arc<Mutex<Router<C>>>,
    state_machines: BTreeMap<u64, StateMachine<C>>,
}

impl<C> TestCluster<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    /// Starts the nodes `node_ids`. Each holds only `membership`, at log index 0, and no vote, so
    /// no node leads until the test triggers an election.
    pub async fn start(node_ids: BTreeSet<u64>, membership: C::Membership) -> Result<Self> {
        let config = Arc::new(
            Config {
                enable_tick: false,
                ..Default::default()
            }
            .validate()?,
        );
        let router = Arc::new(Mutex::new(Router {
            nodes: BTreeMap::new(),
            cut_links: BTreeSet::new(),
        }));
        let mut state_machines = BTreeMap::new();

        for node_id in node_ids {
            let mut log_store = LogStore::<C>::default();
            let initial_log_id = log_id::<C>(0, 0, 0);
            let membership_entry = EntryOf::<C>::new_membership(initial_log_id, membership.clone());
            log_store.blocking_append([membership_entry]).await?;

            let state_machine = StateMachine::<C>::default();
            let network = Network {
                source: node_id,
                router: router.clone(),
            };
            let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine.clone()).await?;

            let mut locked_router = router.lock().unwrap();
            locked_router.nodes.insert(node_id, raft);
            drop(locked_router);

            state_machines.insert(node_id, state_machine);
        }

        Ok(Self { router, state_machines })
    }

    pub fn raft(&self, node_id: u64) -> TestRaft<C> {
        let router = self.router.lock().unwrap();
        router.nodes[&node_id].clone()
    }

    /// Fails every RPC between the two nodes of each link in `links`, and delivers all others.
    pub fn cut_links(&self, links: BTreeSet<(u64, u64)>) {
        let mut router = self.router.lock().unwrap();
        router.cut_links = links;
    }

    pub fn applied(&self, node_id: u64) -> Vec<EntryOf<C>> {
        self.state_machines[&node_id].applied()
    }
}
