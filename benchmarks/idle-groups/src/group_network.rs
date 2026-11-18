use openraft::network::RaftNetworkFactory;
use openraft_multi::GroupNetworkAdapter;

use crate::C;
use crate::stub_followers::StubFollowers;

/// The network factory of one group: it binds the group id to the shared [`StubFollowers`].
pub(crate) struct GroupNetwork {
    followers: StubFollowers,
    group_id: u64,
}

impl GroupNetwork {
    pub(crate) fn new(followers: StubFollowers, group_id: u64) -> Self {
        Self { followers, group_id }
    }
}

impl RaftNetworkFactory<C> for GroupNetwork {
    type Network = GroupNetworkAdapter<C, u64, StubFollowers>;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        GroupNetworkAdapter::new(self.followers.clone(), target, self.group_id)
    }
}
