use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::RaftNetworkFactory;
use openraft::impls::EmptyNode;
use tokio::sync::RwLock;

use crate::Raft;
use crate::TypeConfig;
use crate::connection::Connection;

/// In-process endpoints, but every request and response crosses a byte codec.
#[derive(Clone, Default)]
pub(crate) struct Network(pub(crate) Arc<RwLock<BTreeMap<u64, Raft>>>);

impl Network {
    pub(crate) async fn insert(&self, id: u64, raft: Raft) {
        self.0.write().await.insert(id, raft);
    }

    pub(crate) async fn clear(&self) {
        self.0.write().await.clear();
    }
}

impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = Connection;

    async fn new_client(&mut self, target: u64, _node: &EmptyNode) -> Connection {
        Connection {
            network: self.clone(),
            target,
        }
    }
}
