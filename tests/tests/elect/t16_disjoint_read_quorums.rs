use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_set;
use std::fmt;
use std::future::Future;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use futures::Stream;
use futures::TryStreamExt;
use maplit::btreeset;
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
use openraft::ServerState;
use openraft::Vote;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::errors::MembershipError;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::QuorumBridge;
use openraft::membership::QuorumIntersection;
use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::storage::RaftStateMachine;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use serde::Deserialize;
use serde::Serialize;

use crate::fixtures::ut_harness;

/// A quorum rule that accepts a set of voters when the set contains one of the listed quorums.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct MinimalQuorums(Vec<BTreeSet<u64>>);

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
    fn bridge_to(&self, _other: MinimalQuorums) -> Self {
        unimplemented!("the test never changes the membership")
    }

    fn joint_with(&self, _other: MinimalQuorums) -> Self {
        unimplemented!("the test never changes the membership")
    }
}

/// A membership whose read and write quorum rules differ. Every voter campaigns and receives logs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct GridMembership {
    read: MinimalQuorums,
    write: MinimalQuorums,
}

impl fmt::Display for GridMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{read: {:?}, write: {:?}}}", self.read.0, self.write.0)
    }
}

impl RaftMembership for GridMembership {
    type NodeId = u64;
    type Node = ();
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
        if !is_read_voter && !is_write_voter {
            return None;
        }

        let node_ref = NodeRef {
            node: &(),
            role: NodeRole::Campaigner | NodeRole::ReceiveLogs,
            is_read_voter,
            is_write_voter,
        };
        Some(node_ref)
    }

    type Iter = btree_set::IntoIter<u64>;

    fn node_ids(&self) -> Self::Iter {
        let mut ids: BTreeSet<u64> = self.read.ids().collect();
        ids.extend(self.write.ids());
        ids.into_iter()
    }

    fn new<I>(_read_quorum_set: MinimalQuorums, _write_quorum_set: MinimalQuorums, _nodes: I) -> Self
    where I: IntoIterator<Item = (u64, (), NodeRole)> {
        unimplemented!("the test never changes the membership")
    }

    type Edit = ChangeMembers<u64, ()>;

    fn change(&self, _change: ChangeMembers<u64, ()>) -> Result<Self, MembershipError<u64>> {
        unimplemented!("the test never changes the membership")
    }
}

/// The grid of `openraft/src/docs/protocol/election_quorum.md`: each row is a read quorum and each
/// column is a write quorum. The rows share no voter.
///
/// ```text
///             write {1,3}   write {2,4}
/// read {1,2}       1             2
/// read {3,4}       3             4
/// ```
fn grid() -> GridMembership {
    GridMembership {
        read: MinimalQuorums(vec![btreeset! {1, 2}, btreeset! {3, 4}]),
        write: MinimalQuorums(vec![btreeset! {1, 3}, btreeset! {2, 4}]),
    }
}

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = GridMembership,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = GridMembership,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// A type config that the harness below can run: `StdConfig` or `AdvConfig`.
trait GridConfig
where Self: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = (), Membership = GridMembership>
{
}

impl GridConfig for StdConfig {}
impl GridConfig for AdvConfig {}

#[derive(Clone, Debug, Default)]
struct LogStore<C>(Arc<Mutex<LogStoreInner<C>>>)
where C: GridConfig;

#[derive(Debug, Default)]
struct LogStoreInner<C>
where C: GridConfig
{
    log: BTreeMap<u64, EntryOf<C>>,
    vote: Option<VoteOf<C>>,
}

impl<C> RaftLogReader<C> for LogStore<C>
where
    C: GridConfig,
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
    C: GridConfig,
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
struct StateMachine<C>(Arc<Mutex<StateMachineInner<C>>>)
where C: GridConfig;

#[derive(Debug, Default)]
struct StateMachineInner<C>
where C: GridConfig
{
    applied: Vec<EntryOf<C>>,
    membership: StoredMembershipOf<C>,
}

impl<C> StateMachine<C>
where
    C: GridConfig,
    EntryOf<C>: Clone,
{
    fn applied(&self) -> Vec<EntryOf<C>> {
        let inner = self.0.lock().unwrap();
        inner.applied.clone()
    }
}

impl<C> RaftSnapshotBuilder<C> for StateMachine<C>
where C: GridConfig
{
    type SnapshotData = ();

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, ()>, io::Error> {
        unimplemented!("the test builds no snapshot")
    }
}

impl<C> RaftStateMachine<C> for StateMachine<C>
where C: GridConfig
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

type GridRaft<C> = Raft<C, StateMachine<C>>;

/// Delivers RPCs among the nodes of one test, except between the two nodes of a cut link.
struct Router<C>
where C: GridConfig
{
    nodes: BTreeMap<u64, GridRaft<C>>,
    cut_links: BTreeSet<(u64, u64)>,
}

/// The network that node `source` sends its RPCs through.
struct Network<C>
where C: GridConfig
{
    source: u64,
    router: Arc<Mutex<Router<C>>>,
}

impl<C> RaftNetworkFactory<C> for Network<C>
where C: GridConfig
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

struct Connection<C>
where C: GridConfig
{
    source: u64,
    target: u64,
    router: Arc<Mutex<Router<C>>>,
}

impl<C> Connection<C>
where C: GridConfig
{
    /// Returns the target node, or `Unreachable` if the link to it is cut.
    fn target(&self) -> Result<GridRaft<C>, RPCError<C>> {
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
where C: GridConfig
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

/// Nodes 1 to 4 of the grid membership, connected through one router.
struct GridCluster<C>
where C: GridConfig
{
    router: Arc<Mutex<Router<C>>>,
    state_machines: BTreeMap<u64, StateMachine<C>>,
}

impl<C> GridCluster<C>
where
    C: GridConfig,
    EntryOf<C>: Clone,
{
    /// Starts nodes 1 to 4. Each holds only the grid membership, at log index 0, and no vote, so no
    /// node leads until the test triggers an election.
    async fn start() -> Result<Self> {
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

        for node_id in 1..=4 {
            let mut log_store = LogStore::<C>::default();
            let initial_log_id = log_id::<C>(0, 0, 0);
            let membership_entry = EntryOf::<C>::new_membership(initial_log_id, grid());
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

    fn raft(&self, node_id: u64) -> GridRaft<C> {
        let router = self.router.lock().unwrap();
        router.nodes[&node_id].clone()
    }

    /// Fails every RPC between the two nodes of each link in `links`, and delivers all others.
    fn cut_links(&self, links: BTreeSet<(u64, u64)>) {
        let mut router = self.router.lock().unwrap();
        router.cut_links = links;
    }

    fn applied(&self, node_id: u64) -> Vec<EntryOf<C>> {
        self.state_machines[&node_id].applied()
    }
}

/// The links between the rows `{1,2}` and `{3,4}` of the grid.
fn links_between_rows() -> BTreeSet<(u64, u64)> {
    btreeset! {(1, 3), (1, 4), (2, 3), (2, 4)}
}

/// With standard leader IDs, a row of the grid is a read quorum but not a write quorum, so it
/// cannot elect a leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn standard_leader_ids_need_a_read_and_a_write_quorum() -> Result<()> {
    let cluster = GridCluster::<StdConfig>::start().await?;

    tracing::info!("--- cut the rows {{1,2}} and {{3,4}} apart, then campaign at nodes 1 and 4 in term 1");
    {
        cluster.cut_links(links_between_rows());
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(4).trigger().elect(false).await?;
    }

    tracing::info!("--- node 2 votes for node 1 and node 3 for node 4, yet neither candidate becomes leader");
    {
        cluster.raft(2).wait(timeout()).vote(Vote::new(1, 1), "node 2 votes for node 1").await?;
        cluster.raft(3).wait(timeout()).vote(Vote::new(1, 4), "node 3 votes for node 4").await?;

        StdConfig::sleep(Duration::from_millis(500)).await;

        for node_id in [1, 4] {
            let raft = cluster.raft(node_id);
            let metrics = raft.metrics().borrow_watched().clone();
            assert_eq!(ServerState::Candidate, metrics.state, "node {node_id}");
        }
    }

    tracing::info!("--- restore the links; node 4 wins term 2 with votes from both rows and commits a write");
    {
        cluster.cut_links(btreeset! {});
        cluster.raft(4).trigger().elect(false).await?;
        cluster.raft(4).wait(timeout()).applied_index(Some(1), "node 4 commits its blank entry").await?;

        let response = cluster.raft(4).client_write("from-4".to_string()).await?;
        assert_eq!(log_id::<StdConfig>(2, 4, 2), response.log_id);
    }

    tracing::info!("--- every node applies the same entries");
    {
        let want = vec![
            EntryOf::<StdConfig>::new_membership(log_id::<StdConfig>(0, 0, 0), grid()),
            EntryOf::<StdConfig>::new_blank(log_id::<StdConfig>(2, 4, 1)),
            EntryOf::<StdConfig>::new_normal(log_id::<StdConfig>(2, 4, 2), "from-4".to_string()),
        ];
        for node_id in 1..=4 {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(2), "apply the write").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

/// With advanced leader IDs, each row of the grid elects a leader in the same term. Every write
/// quorum spans both rows, so neither leader commits or accepts a write while the rows are cut
/// apart. After the links return, only the entries of the greater leader ID commit.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn advanced_leader_ids_elect_by_read_quorum_and_commit_by_write_quorum() -> Result<()> {
    let cluster = GridCluster::<AdvConfig>::start().await?;

    tracing::info!("--- cut the rows {{1,2}} and {{3,4}} apart; nodes 1 and 4 both win term 1");
    {
        cluster.cut_links(links_between_rows());
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(4).trigger().elect(false).await?;

        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "row {1,2} elects node 1").await?;
        cluster.raft(4).wait(timeout()).state(ServerState::Leader, "row {3,4} elects node 4").await?;
    }

    tracing::info!("--- each row stores the blank entry of its own leader at index 1");
    {
        cluster.raft(2).wait(timeout()).log_index(Some(1), "node 2 stores node 1's blank entry").await?;
        cluster.raft(3).wait(timeout()).log_index(Some(1), "node 3 stores node 4's blank entry").await?;
    }

    tracing::info!("--- no write quorum has acknowledged either leader, so both reject writes");
    {
        for node_id in [1, 4] {
            let data = format!("from-{node_id}");
            let result = cluster.raft(node_id).client_write(data).await;
            let err = result.unwrap_err();
            let forward = ForwardToLeader {
                leader_id: None,
                leader_node: None,
                reason: ForwardReason::LeaseExpired,
            };
            let want = RaftError::APIError(ClientWriteError::ForwardToLeader(forward));
            assert_eq!(want, err, "node {node_id}");
        }
    }

    tracing::info!("--- restore the links; node 4, the greater leader ID, commits its blank entry and a write");
    {
        cluster.cut_links(btreeset! {});
        cluster.raft(4).wait(timeout()).applied_index(Some(1), "node 4 commits its blank entry").await?;

        let response = cluster.raft(4).client_write("from-4".to_string()).await?;
        assert_eq!(log_id::<AdvConfig>(1, 4, 2), response.log_id);
    }

    tracing::info!("--- every node applies the entries of node 4, and none of node 1");
    {
        let want = vec![
            EntryOf::<AdvConfig>::new_membership(log_id::<AdvConfig>(0, 0, 0), grid()),
            EntryOf::<AdvConfig>::new_blank(log_id::<AdvConfig>(1, 4, 1)),
            EntryOf::<AdvConfig>::new_normal(log_id::<AdvConfig>(1, 4, 2), "from-4".to_string()),
        ];
        for node_id in 1..=4 {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(2), "apply the write of node 4").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
