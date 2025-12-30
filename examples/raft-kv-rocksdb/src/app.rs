use std::collections::BTreeMap;
use std::sync::Arc;

use futures::lock::Mutex;
use openraft::Config;
use openraft_legacy::ChunkedRaft;

use crate::NodeId;
use crate::TypeConfig;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub addr: String,
    pub raft: ChunkedRaft<TypeConfig>,
    pub key_values: Arc<Mutex<BTreeMap<String, String>>>,
    pub config: Arc<Config>,
}
