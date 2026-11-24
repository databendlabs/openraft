use openraft::network::RaftNetworkFactory;
use openraft_multi::GroupNetworkAdapter;

use crate::C;
use crate::local_members::LocalMembers;

/// The network factory of one group: it binds the group id to the router that every group shares.
pub(crate) struct GroupNetwork {
    router: LocalMembers,
    group_id: u64,
}

impl GroupNetwork {
    pub(crate) fn new(router: LocalMembers, group_id: u64) -> Self {
        Self { router, group_id }
    }
}

impl RaftNetworkFactory<C> for GroupNetwork {
    type Network = GroupNetworkAdapter<C, u64, LocalMembers>;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        GroupNetworkAdapter::new(self.router.clone(), target, self.group_id)
    }
}
