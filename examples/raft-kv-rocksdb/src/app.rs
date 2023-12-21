use std::collections::BTreeMap;
use std::sync::Arc;

use async_std::sync::RwLock;
use openraft::Config;

use crate::ExampleRaft;
use crate::NodeId;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub api_addr: String,
    pub rcp_addr: String,
    pub raft: ExampleRaft,
    pub key_values: Arc<RwLock<BTreeMap<String, String>>>,
    pub config: Arc<Config>,
}
