use std::sync::Weak;

use openraft::RaftNetworkFactory;
use openraft::RaftTypeConfig;
use openraft::storage::RaftStateMachine;

use crate::Connection;
use crate::Registry;

/// Builds the [`Connection`]s that one node sends its RPCs through.
///
/// Get it from [`Registry::network_factory()`] and pass it to `Raft::new()`. The node metadata is
/// not used: a connection finds its target in the registry by node ID.
pub struct NetworkFactory<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    source: C::NodeId,
    registry: Weak<Registry<C, SM>>,
}

impl<C, SM> NetworkFactory<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(source: C::NodeId, registry: Weak<Registry<C, SM>>) -> Self {
        Self { source, registry }
    }
}

impl<C, SM> RaftNetworkFactory<C> for NetworkFactory<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    type Network = Connection<C, SM>;

    async fn new_client(&mut self, target: C::NodeId, _node: &C::Node) -> Self::Network {
        Connection::new(self.source.clone(), target, self.registry.clone())
    }
}
