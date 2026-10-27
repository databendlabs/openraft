use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::OptionalSend;
use openraft::RaftNetworkFactory;
use openraft::alias::SnapshotOf;
use openraft::alias::VoteOf;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;

use crate::fixtures::custom_membership::test_cluster::TestConfig;
use crate::fixtures::custom_membership::test_cluster::TestRaft;

/// Delivers RPCs among the nodes of one test, except between the two nodes of a cut link.
pub struct Router<C>
where C: TestConfig
{
    pub(super) nodes: BTreeMap<u64, TestRaft<C>>,
    pub(super) cut_links: BTreeSet<(u64, u64)>,
}

/// The network that node `source` sends its RPCs through.
pub struct Network<C>
where C: TestConfig
{
    pub(super) source: u64,
    pub(super) router: Arc<Mutex<Router<C>>>,
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
