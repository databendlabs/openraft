use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::Config;
use openraft::type_config::alias::MutexOf;

use crate::NodeId;
use crate::TypeConfig;
use crate::typ::Raft;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub addr: String,
    pub raft: Raft,
    pub key_values: Arc<MutexOf<TypeConfig, BTreeMap<String, String>>>,
    pub config: Arc<Config>,
}
