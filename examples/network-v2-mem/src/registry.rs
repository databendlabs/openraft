use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::storage::RaftStateMachine;

use crate::NetworkFactory;

/// The [`Raft`] handles of one in-process cluster, by node ID.
///
/// The application that creates the nodes owns the registry in an [`Arc`]. A [`NetworkFactory`]
/// keeps only a weak reference to it: every `Raft` handle owns its network factory, so a strong
/// reference would keep the registry and every node alive in a cycle. After the owner drops the
/// registry, every RPC fails with `Unreachable`.
pub struct Registry<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    nodes: Mutex<BTreeMap<C::NodeId, Raft<C, SM>>>,
}

impl<C, SM> Default for Registry<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn default() -> Self {
        Self {
            nodes: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<C, SM> Registry<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Returns the network factory of node `source`, to pass to `Raft::new()`.
    ///
    /// Its connections send RPCs from `source` to the nodes of this registry.
    pub fn network_factory(self: &Arc<Self>, source: C::NodeId) -> NetworkFactory<C, SM> {
        NetworkFactory::new(source, Arc::downgrade(self))
    }

    /// Registers `raft` as node `id`, and returns the handle that was registered as `id` before.
    pub fn register(&self, id: C::NodeId, raft: Raft<C, SM>) -> Option<Raft<C, SM>> {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.insert(id, raft)
    }

    /// Returns a clone of the handle registered as `id`.
    ///
    /// The lock is released when this returns, so a caller awaits an RPC on the clone without
    /// blocking the other connections.
    pub(crate) fn get(&self, id: &C::NodeId) -> Option<Raft<C, SM>> {
        let nodes = self.nodes.lock().unwrap();
        nodes.get(id).cloned()
    }
}
