use openraft::network::RaftNetworkFactory;
use openraft_multi::GroupNetworkAdapter;
use openraft_multi::GroupRouter;

use crate::C;

/// The network factory of one group: it binds the group id to the router that every group shares.
pub(crate) struct GroupNetwork<R> {
    router: R,
    group_id: u64,
}

impl<R> GroupNetwork<R> {
    pub(crate) fn new(router: R, group_id: u64) -> Self {
        Self { router, group_id }
    }
}

impl<R> RaftNetworkFactory<C> for GroupNetwork<R>
where R: GroupRouter<C, u64>
{
    type Network = GroupNetworkAdapter<C, u64, R>;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        GroupNetworkAdapter::new(self.router.clone(), target, self.group_id)
    }
}
