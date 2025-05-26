use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::Config;
use tokio::sync::RwLock;

use crate::typ::Raft;
use crate::NodeId;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub addr: String,
    pub raft: Raft,
    pub key_values: Arc<RwLock<BTreeMap<String, String>>>,
    pub config: Arc<Config>,
}
