use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::storage::RaftStateMachine;

use crate::NetworkFactory;

/// The [`Raft`] handles of one in-process cluster, by node ID, and the nodes cut off from it.
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

    /// The nodes that [`isolate()`](Self::isolate) cut off from every other node.
    isolated: Mutex<BTreeSet<C::NodeId>>,
}

impl<C, SM> Default for Registry<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn default() -> Self {
        Self {
            nodes: Mutex::new(BTreeMap::new()),
            isolated: Mutex::new(BTreeSet::new()),
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

    /// Cuts each node of `ids` off from every other node, in both directions.
    ///
    /// An RPC that starts while its source or its target is isolated fails with `Unreachable`,
    /// and an RPC that started before completes. An isolated node keeps running with its own
    /// stores, so [`heal()`](Self::heal) reconnects the same node in the state it reached.
    pub fn isolate(&self, ids: impl IntoIterator<Item = C::NodeId>) {
        let mut isolated = self.isolated.lock().unwrap();
        isolated.extend(ids);
    }

    /// Reconnects every isolated node, and returns the nodes that it reconnected.
    ///
    /// A leader backs off before it retries a node that was unreachable. To retry the
    /// reconnected nodes at once, pass them to `Trigger::reset_backoff()` on the leader.
    pub fn heal(&self) -> BTreeSet<C::NodeId> {
        let mut isolated = self.isolated.lock().unwrap();
        std::mem::take(&mut *isolated)
    }

    /// Returns whether node `id` is isolated.
    pub fn is_isolated(&self, id: &C::NodeId) -> bool {
        let isolated = self.isolated.lock().unwrap();
        isolated.contains(id)
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
