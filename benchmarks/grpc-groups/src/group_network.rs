use openraft::network::RaftNetworkFactory;
use openraft_multi::GroupNetworkAdapter;
use raft_kv_memstore_grpc::protobuf as raftpb;

use crate::C;
use crate::router::Router;

/// The network factory of one group: it binds the group id to the router that all groups of the
/// node share.
pub(crate) struct GroupNetwork {
    router: Router,
    group_id: u64,
}

impl GroupNetwork {
    pub(crate) fn new(router: Router, group_id: u64) -> Self {
        Self { router, group_id }
    }
}

impl RaftNetworkFactory<C> for GroupNetwork {
    type Network = GroupNetworkAdapter<C, u64, Router>;

    async fn new_client(&mut self, target: u64, _node: &raftpb::Node) -> Self::Network {
        GroupNetworkAdapter::new(self.router.clone(), target, self.group_id)
    }
}
